// Mirrors crates/ripple-core/src/model.rs and api.rs. Keep the two in step.

export type Jid = string;

export type ChatKind = "direct" | "group" | "announcement";
export type Status = "failed" | "pending" | "sent" | "delivered" | "read" | "played";
export type MediaState = "remote" | "downloading" | "ready" | "failed";
export type Role = "member" | "admin" | "owner";

export interface Preview {
  id: string;
  text: string;
  kind: string;
  fromMe: boolean;
  sender: Jid | null;
  status: Status;
  ts: number;
}

export interface Chat {
  id: Jid;
  kind: ChatKind;
  name: string;
  avatar: string | null;
  last: Preview | null;
  ts: number;
  unread: number;
  unreadMentions: number;
  markedUnread: boolean;
  pinned: number;
  archived: boolean;
  mutedUntil: number;
  favorite: boolean;
  draft: string | null;
  ephemeral: number;
  community: Jid | null;
  readOnly: boolean;
}

export interface Contact {
  id: Jid;
  name: string | null;
  pushName: string | null;
  verifiedName: string | null;
  about: string | null;
  avatar: string | null;
  blocked: boolean;
  isMe: boolean;
  isBusiness: boolean;
}

export interface Participant {
  id: Jid;
  role: Role;
}

export interface Group {
  id: Jid;
  subject: string;
  description: string | null;
  owner: Jid | null;
  createdAt: number;
  participants: Participant[];
  announce: boolean;
  locked: boolean;
  approval: boolean;
  membersCanAdd: boolean;
  ephemeral: number;
  inviteCode: string | null;
  isCommunity: boolean;
  isAnnouncement: boolean;
  parent: Jid | null;
  pending: Jid[];
  joined: boolean;
  avatar: string | null;
  size: number;
}

export interface Media {
  token: string | null;
  mime: string;
  size: number;
  state: MediaState;
}

export interface LinkPreview {
  url: string;
  title: string;
  description: string;
  site: string;
  thumb: string | null;
}

export interface PollOption {
  name: string;
  voters: Jid[];
}

export interface ContactCard {
  name: string;
  vcard: string;
  phone: string | null;
  jid: Jid | null;
}

export interface EventResponses {
  going: Jid[];
  maybe: Jid[];
  declined: Jid[];
}

export type Content =
  | { type: "text"; text: string; preview: LinkPreview | null }
  | { type: "image"; media: Media; caption: string; width: number; height: number; thumb: string | null }
  | { type: "video"; media: Media; caption: string; width: number; height: number; duration: number; gif: boolean; thumb: string | null }
  | { type: "audio"; media: Media; duration: number; voice: boolean; waveform: number[] }
  | { type: "document"; media: Media; name: string; caption: string; pages: number; thumb: string | null }
  | { type: "sticker"; media: Media; width: number; height: number; animated: boolean }
  | { type: "location"; lat: number; lng: number; name: string; address: string; live: boolean; thumb: string | null }
  | { type: "contacts"; cards: ContactCard[] }
  | { type: "poll"; question: string; options: PollOption[]; selectable: number }
  | { type: "event"; name: string; description: string; location: string; start: number; end: number; canceled: boolean; responses: EventResponses }
  | { type: "system"; event: string; actor: Jid | null; targets: Jid[]; text: string }
  | { type: "call"; video: boolean; missed: boolean; duration: number }
  | { type: "deleted" }
  | { type: "unsupported"; note: string };

export type ContentOf<T extends Content["type"]> = Extract<Content, { type: T }>;

export interface Reaction {
  emoji: string;
  sender: Jid;
  ts: number;
}

export interface Quote {
  id: string;
  sender: Jid;
  text: string;
  kind: string;
  thumb: string | null;
}

export interface Message {
  id: string;
  chat: Jid;
  seq: number;
  sender: Jid;
  fromMe: boolean;
  ts: number;
  content: Content;
  status: Status;
  quote: Quote | null;
  reactions: Reaction[];
  mentions: Jid[];
  starred: boolean;
  pinnedUntil: number;
  edited: number;
  forwarded: number;
  viewOnce: boolean;
  expires: number;
  album: string | null;
}

export interface Receipt {
  user: Jid;
  delivered: number;
  read: number;
  played: number;
}

export type Session =
  | { state: "welcome" }
  | { state: "pairing"; qr: string | null; code: string | null; expires: number }
  | { state: "syncing"; progress: number }
  | { state: "connecting" }
  | { state: "online" }
  | { state: "offline" }
  | { state: "loggedOut"; reason: string };

export interface Settings {
  theme: "system" | "light" | "dark";
  accent: string;
  font: string;
  design: number;
  textScale: number;
  compact: boolean;
  enterSends: boolean;
  notifications: boolean;
  notificationPreview: boolean;
  notificationSound: boolean;
  autoImages: boolean;
  autoAudio: boolean;
  autoVideo: boolean;
  autoDocuments: boolean;
  linkPreviews: boolean;
  closeToTray: boolean;
  autostart: boolean;
  spellcheck: boolean;
  sidebarWidth: number;
  quickReactions: string[];
}

export interface Privacy {
  lastSeen: string;
  online: string;
  profilePhoto: string;
  about: string;
  readReceipts: boolean;
  groupAdd: string;
  defaultEphemeral: number;
}

export interface Sticker {
  /** Content hash; the same id WhatsApp uses to sync favourites and recents. */
  id: string;
  /** Local file, or null while a synced sticker is still downloading. */
  token: string | null;
  width: number;
  height: number;
  animated: boolean;
  favorite: boolean;
  used: number;
  remote: unknown | null;
  /** WhatsApp no longer has the file and the phone couldn't upload it again. */
  missing: boolean;
}

export interface Cursor {
  ts: number;
  seq: number;
}

export interface Attachment {
  path?: string | null;
  data?: string | null;
  name: string;
  mime: string;
  width?: number;
  height?: number;
  duration?: number;
  thumb?: string | null;
}

export type Outgoing =
  | { type: "text"; text: string; preview?: LinkPreview | null }
  | { type: "media"; file: Attachment; kind: string; caption?: string; viewOnce?: boolean; album?: string | null }
  | { type: "sticker"; id: string }
  | { type: "location"; lat: number; lng: number; name?: string; address?: string }
  | { type: "contacts"; jids: Jid[] }
  | { type: "poll"; question: string; options: string[]; selectable: number }
  | { type: "event"; name: string; description?: string; location?: string; start: number; end?: number };

export type UiEvent =
  | { type: "reset" }
  | { type: "session"; session: Session }
  | { type: "chat"; chat: Chat }
  | { type: "chatRemoved"; id: Jid }
  | { type: "message"; message: Message; fresh: boolean }
  | { type: "messageRemoved"; chat: Jid; id: string }
  | { type: "history"; chat: Jid }
  | { type: "contact"; contact: Contact }
  | { type: "group"; group: Group }
  | { type: "groupRemoved"; id: Jid }
  | { type: "typing"; chat: Jid; user: Jid; state: string }
  | { type: "presence"; jid: Jid; online: boolean; lastSeen: number }
  | { type: "settings"; settings: Settings }
  | { type: "privacy"; privacy: Privacy }
  | { type: "me"; contact: Contact }
  | { type: "notify"; chat: Jid; id: string; title: string; body: string; avatar: string | null }
  | { type: "call"; id: string; from: Jid; video: boolean; ringing: boolean }
  | { type: "recording"; level: number; elapsed: number }
  | { type: "progress"; chat: Jid; id: string; progress: number }
  | { type: "toast"; tone: string; text: string }
  | { type: "badge"; count: number }
  | { type: "stickers" };

export interface InitData {
  session: Session;
  me: Contact | null;
  settings: Settings;
  privacy: Privacy;
  chats: Chat[];
  contacts: Contact[];
  groups: Group[];
  demo: boolean;
  platform: string;
  version: string;
}
