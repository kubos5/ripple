//! Link previews: fetch a page, read its Open Graph tags and thumbnail.
//!
//! Runs only for URLs the user typed into the composer, and only when link
//! previews are enabled in settings.

use std::time::Duration;

use crate::model::LinkPreview;

const MAX_HTML: u64 = 700_000;
const MAX_IMAGE: u64 = 3_000_000;

/// Blocking; call from `spawn_blocking`.
pub fn fetch(url: &str) -> Option<LinkPreview> {
    let url = url.trim();
    let host = host_of(url)?;
    if is_private_host(&host) {
        return None;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(6)))
        .max_redirects(4)
        .user_agent("Mozilla/5.0 (compatible; RipplePreview/0.1)")
        .build()
        .into();
    let mut res = agent.get(url).header("Accept", "text/html,application/xhtml+xml").call().ok()?;
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    if !ctype.is_empty() && !ctype.contains("html") {
        return None;
    }
    let bytes = res.body_mut().with_config().limit(MAX_HTML).read_to_vec().ok()?;
    let html = String::from_utf8_lossy(&bytes);
    let mut p = parse(&html, url);
    if p.title.is_empty() {
        return None;
    }
    if p.site.is_empty() {
        p.site = host.trim_start_matches("www.").to_string();
    }
    if let Some(img) = p.thumb.take() {
        p.thumb = agent
            .get(&img)
            .call()
            .ok()
            .and_then(|mut r| r.body_mut().with_config().limit(MAX_IMAGE).read_to_vec().ok())
            .and_then(|b| crate::media::thumb_from_bytes(&b, 192));
    }
    Some(p)
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?.to_ascii_lowercase();
    if host.is_empty() || !host.contains('.') { None } else { Some(host) }
}

fn is_private_host(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.starts_with("0.")
        || (host.starts_with("172.")
            && host.split('.').nth(1).and_then(|o| o.parse::<u8>().ok()).is_some_and(|o| (16..32).contains(&o)))
}

/// Extracts title, description, site name and image URL (left in `thumb`) from HTML.
pub fn parse(html: &str, base: &str) -> LinkPreview {
    let mut p = LinkPreview { url: base.to_string(), ..Default::default() };
    let mut fallback_desc = String::new();
    let mut tw_title = String::new();
    let mut image = String::new();
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<meta") {
        let start = at + i;
        let Some(end) = lower[start..].find('>') else { break };
        let tag = &html[start..start + end];
        at = start + end;
        let key = attr(tag, "property").or_else(|| attr(tag, "name")).unwrap_or_default().to_ascii_lowercase();
        let Some(content) = attr(tag, "content") else { continue };
        let content = decode_entities(content.trim());
        match key.as_str() {
            "og:title" => p.title = content,
            "og:description" => p.description = content,
            "og:site_name" => p.site = content,
            "og:image" | "og:image:url" | "og:image:secure_url" if image.is_empty() => image = content,
            "twitter:image" if image.is_empty() => image = content,
            "twitter:title" => tw_title = content,
            "description" | "twitter:description" if fallback_desc.is_empty() => fallback_desc = content,
            _ => {}
        }
    }
    if p.title.is_empty() {
        p.title = tw_title;
    }
    if p.title.is_empty()
        && let Some(s) = lower.find("<title")
            && let (Some(open), Some(close)) = (lower[s..].find('>'), lower[s..].find("</title"))
                && close > open {
                    p.title = decode_entities(html[s + open + 1..s + close].trim());
                }
    if p.description.is_empty() {
        p.description = fallback_desc;
    }
    p.title = crate::model::truncate(&collapse(&p.title), 160);
    p.description = crate::model::truncate(&collapse(&p.description), 300);
    if !image.is_empty() {
        p.thumb = absolutize(&image, base);
    }
    p
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads one attribute value out of a tag's source text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let idx = from + i;
        from = idx + name.len();
        let before_ok = idx == 0 || lower.as_bytes()[idx - 1].is_ascii_whitespace();
        let rest = tag[idx + name.len()..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        return match rest.chars().next()? {
            q @ ('"' | '\'') => rest[1..].find(q).map(|e| &rest[1..1 + e]),
            _ => Some(rest.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("")),
        };
    }
    None
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "rdquo" => Some('”'),
            "ldquo" => Some('“'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn absolutize(src: &str, base: &str) -> Option<String> {
    if src.starts_with("https://") || src.starts_with("http://") {
        return Some(src.to_string());
    }
    let scheme_end = base.find("://")?;
    let scheme = &base[..scheme_end];
    let after = &base[scheme_end + 3..];
    let host = after.split(['/', '?', '#']).next()?;
    if let Some(rest) = src.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if src.starts_with('/') {
        return Some(format!("{scheme}://{host}{src}"));
    }
    let path = after[host.len()..].split(['?', '#']).next().unwrap_or("");
    let dir = path.rfind('/').map(|i| &path[..=i]).unwrap_or("/");
    Some(format!("{scheme}://{host}{dir}{src}"))
}

/// Finds the first http(s) URL in a piece of text.
pub fn first_url(text: &str) -> Option<&str> {
    text.split_whitespace()
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(|w| w.trim_end_matches(['.', ',', ')', '!', '?', ';', ':']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_graph() {
        let html = r#"<html><head>
            <title>Fallback &amp; title</title>
            <meta property="og:title" content="Tide tables &#8212; Harbour">
            <meta content='Daily high &amp; low water' property='og:description'>
            <meta property="og:image" content="/img/cover.jpg">
            <meta name="og:site_name" content=Harbourly>
        </head></html>"#;
        let p = parse(html, "https://example.org/tides/today?x=1");
        assert_eq!(p.title, "Tide tables — Harbour");
        assert_eq!(p.description, "Daily high & low water");
        assert_eq!(p.site, "Harbourly");
        assert_eq!(p.thumb.as_deref(), Some("https://example.org/img/cover.jpg"));
    }

    #[test]
    fn falls_back_to_title_and_description() {
        let html = "<head><TITLE>  Plain\n page </TITLE><meta name=\"description\" content=\"About it\"></head>";
        let p = parse(html, "https://example.org/");
        assert_eq!(p.title, "Plain page");
        assert_eq!(p.description, "About it");
        assert!(p.thumb.is_none());
    }

    #[test]
    fn url_helpers() {
        assert_eq!(host_of("https://user@Example.org:8443/a").as_deref(), Some("example.org"));
        assert!(host_of("ftp://example.org").is_none());
        assert!(is_private_host("192.168.1.4") && is_private_host("172.20.0.1") && is_private_host("localhost"));
        assert!(!is_private_host("172.40.0.1") && !is_private_host("example.org"));
        assert_eq!(absolutize("//cdn.x.io/a.png", "https://x.io/p").as_deref(), Some("https://cdn.x.io/a.png"));
        assert_eq!(absolutize("b.png", "https://x.io/dir/page").as_deref(), Some("https://x.io/dir/b.png"));
        assert_eq!(first_url("see https://x.io/a, ok"), Some("https://x.io/a"));
        assert_eq!(decode_entities("a &amp; b &#39;c&#x27; &bogus; &"), "a & b 'c' &bogus; &");
    }
}
