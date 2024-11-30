import { createSignal } from "solid-js"
import Loading from "../Components/Loading"
import { invoke } from "@tauri-apps/api/core"
import { Event, listen } from "@tauri-apps/api/event"
import AcceptFiles from "./AcceptFiles"
import { useNavigate } from "@solidjs/router"
import TransferFiles from "./TransferFiles"
import { window } from "@tauri-apps/api"
import { Window } from "@tauri-apps/api/window"

import {
    isPermissionGranted,
    requestPermission,
    sendNotification,
} from "@tauri-apps/plugin-notification"
import { store } from "../App"

type ReceiveState =
    | "connecting-to-server"
    | "connecting-to-sender"
    | "waiting-for-files"
    | "files-offered"
    | "downloading-files"

interface FilesOfferedEvent {
    /// name, size, isDir
    files: [string, number, boolean][]
}

interface ReceiveProps {
    code: string
    onError(error: string): void
}

function Receive(props: ReceiveProps) {
    const [state, setState] = createSignal<ReceiveState>("connecting-to-server")
    const [files, setFiles] = createSignal<[string, number, boolean][]>([])

    invoke("download_files", {
        code: props.code,
        serverAddr: store.roundezvousAddr,
    }).catch((e: string) => {
        props.onError(e)
    })

    listen("server-connected", (_) => {
        setState("connecting-to-sender")
    })

    listen("receiver-connected", (_) => {
        setState("waiting-for-files")
    })

    listen("files-offered", (event: Event<FilesOfferedEvent>) => {
        let files = event.payload.files
        setState("files-offered")
        setFiles(files)
    })

    return (
        <div class="receive">
            {state() == "connecting-to-server" ? (
                <Loading text="Connecting to server..." />
            ) : state() == "waiting-for-files" ? (
                <Loading text="Waiting for files..." />
            ) : state() == "connecting-to-sender" ? (
                <Loading text="Connecting to sender..." />
            ) : state() == "files-offered" ? (
                <AcceptFiles
                    files={files()}
                    acceptFiles={(path) => {
                        if (path) {
                            setState("downloading-files")
                            console.log("Accepting files at", path)
                            Window.getCurrent().emit("accept-files", path)
                        } else {
                            Window.getCurrent().emit("accept-files", "")
                            location.reload()
                        }
                    }}
                />
            ) : state() == "downloading-files" ? (
                <TransferFiles
                    files={files()}
                    type="receive"
                    onCancel={() => invoke("exit", { code: 0 })}
                    onComplete={() => {
                        sendNotification({
                            title: "Quic send",
                            body: "Transfer completed",
                        })
                        location.reload()
                    }}
                />
            ) : null}
        </div>
    )
}

export default Receive
