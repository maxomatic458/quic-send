import { createSignal } from "solid-js";
import logo from "./assets/logo.svg";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import { createStore } from "solid-js/store";
import { Route, Router } from "@solidjs/router";
import Main from "./Pages/Main";
import Send from "./Pages/Send";
import Receive from "./Pages/Receive";
import Connect from "./Pages/Connect";

interface SendArgs {
    /// Files to send
    files: string[]
}

interface RecvArgs {
    /// Resume interrupted transfer
    resume: boolean
    /// Output directory
    output: string
}

type AppState = SendArgs | RecvArgs

const [store, setStore] = createStore<AppState>(
    { files: [] }
);

function App() {
    return <Router>
        <Route path="/" component={Main} />
        <Route path="/send" component={Send} />
        <Route path="/recv" component={Receive} />
        <Route path="/conn" component={Connect} />
    </Router>
}

export default App;
