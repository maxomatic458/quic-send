import { createSignal } from "solid-js";
import logo from "./assets/logo.svg";
import { invoke } from "@tauri-apps/api/tauri";
import { listen } from "@tauri-apps/api/event";
import "../App.css";
import { FileDropEvent } from "@tauri-apps/api/window";
import { Event } from "@tauri-apps/api/event";
import FilesToSendList from "../Components/FilesToSend";

function Send() {
    const [filesToSend, setFilesToSend] = createSignal<string[]>([]);
    // extract the first files from the query string
    const urlParams = new URLSearchParams(window.location.search);
    const file = urlParams.get('files');

    if (file) {
        const files = JSON.parse(file);
        setFilesToSend(files);
    }

    console.log(filesToSend());

    return (
        <div>
            <div class="send-header">
                <a href="/" class="back-button">Back</a>
                <div class="files-to-send-text">Files to Send</div>
            </div>
            <div class="files-to-send-list"> 
                <FilesToSendList initialFilePaths={filesToSend()} />
            </div>
        </div>
    );
}

export default Send;