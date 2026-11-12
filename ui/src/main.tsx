import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/shell.css";
import "./styles/convo.css";
import "./styles/panels.css";
import { render } from "solid-js/web";
import { App } from "./App";

render(() => <App />, document.getElementById("root")!);
