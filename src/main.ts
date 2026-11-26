import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { installAppChrome } from "./lib/browser-chrome";

installAppChrome();

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");

export default mount(App, { target });
