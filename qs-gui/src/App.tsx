import { createSignal } from "solid-js";
import logo from "./assets/logo.svg";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import { createStore } from "solid-js/store";
import { Route, Router } from "@solidjs/router";
import Main from "./Pages/main";
import Send from "./Pages/Send";
import Receive from "./Pages/Receive";

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
    </Router>
}

export default App;
