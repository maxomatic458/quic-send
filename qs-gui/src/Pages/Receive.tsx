import { createSignal, onCleanup } from "solid-js"
import Loading from "../Components/Loading"
import { invoke } from "@tauri-apps/api/core"
import { Event, listen } from "@tauri-apps/api/event"
import AcceptFiles from "./AcceptFiles"
import TransferFiles from "./TransferFiles"
import { Window } from "@tauri-apps/api/window"

import { sendNotification } from "@tauri-apps/plugin-notification"
import { setStore, store } from "../App"
import {
    ACCEPT_FILES_EVENT,
    CONNECTED_TO_SERVER_EVENT,
    CONNECTED_WITH_CONN_TYPE,
    FILES_OFFERED_EVENT,
} from "../events"

export enum ReceiveState {
    ConnectingToServer = "R_connecting-to-server",
    ConnectingToSender = "R_connecting-to-sender",
    WaitingForFiles = "R_waiting-for-files",
    FilesOffered = "R_files-offered",
    DownloadingFiles = "R_downloading-files",
}

interface FilesOfferedEvent {
    /// name, size, isDir
    files: [string, number, boolean][]
}

interface ReceiveProps {
    code: string
    onError(error: string): void
}

function Receive(props: ReceiveProps) {
    const [files, setFiles] = createSignal<[string, number, boolean][]>([])

    invoke("download_files", {
        ticket: props.code,
    }).catch((e: string) => {
        props.onError(e)
    })

    const unlisten1 = listen(CONNECTED_TO_SERVER_EVENT, (_) => {
        setStore("currentState", ReceiveState.ConnectingToSender)
    })

    const unlisten2 = listen(
        CONNECTED_WITH_CONN_TYPE,
        (_conn_type: Event<string>) => {
            setStore("currentState", ReceiveState.WaitingForFiles)
        },
    )

    const unlisten3 = listen(
        FILES_OFFERED_EVENT,
        (event: Event<FilesOfferedEvent>) => {
            let files = event.payload.files
            setStore("currentState", ReceiveState.FilesOffered)
            setFiles(files)
        },
    )

    onCleanup(async () => {
        ;(await unlisten1)()
        ;(await unlisten2)()
        ;(await unlisten3)()
    })

    return (
        <div class="receive">
            {store.currentState === ReceiveState.ConnectingToServer ? (
                <Loading text="Connecting to server..." />
            ) : store.currentState === ReceiveState.WaitingForFiles ? (
                <Loading text="Waiting for files..." />
            ) : store.currentState === ReceiveState.ConnectingToSender ? (
                <Loading text="Connecting to sender..." />
            ) : store.currentState === ReceiveState.FilesOffered ? (
                <AcceptFiles
                    files={files()}
                    acceptFiles={(path) => {
                        if (path) {
                            setStore(
                                "currentState",
                                ReceiveState.DownloadingFiles,
                            )
                            console.log("Accepting files at", path)
                            Window.getCurrent().emit(ACCEPT_FILES_EVENT, path)
                        } else {
                            Window.getCurrent().emit(ACCEPT_FILES_EVENT, "")
                            setStore("currentState", null)
                        }
                    }}
                />
            ) : store.currentState === ReceiveState.DownloadingFiles ? (
                <TransferFiles
                    files={files()}
                    type="receive"
                    onComplete={() => {
                        sendNotification({
                            title: "Quic send",
                            body: "Transfer completed",
                        })
                        setStore("currentState", null)
                    }}
                />
            ) : null}
        </div>
    )
}

export default Receive
