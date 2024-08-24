import { createSignal } from "solid-js";
import logo from "./assets/logo.svg";
import { invoke } from "@tauri-apps/api/tauri";
import { listen } from "@tauri-apps/api/event";
import "../App.css";

// ! When changing this also change this in qs-core/src/lib.rs
// TODO: maybe fetch this from rust?
const codeLength = 8;

function Receive() {
    const [sessionCode, setSessionCode] = createSignal<string>("");

    return (
        <div class="receive-page">
            <div class="files-to-send-text">Enter the session code</div>
            <div class="receive-lower-page-container">
                <div class="input-container">
                    <input
                        class="receive-code-input"
                        maxLength={codeLength}
                        value={sessionCode()}
                        onInput={(e) => setSessionCode(e.currentTarget.value)}
                    />
                </div>
            </div>
        </div>
    );
}

export default Receive;
