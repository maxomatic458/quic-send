import { invoke } from "@tauri-apps/api/core"
import { Event, listen } from "@tauri-apps/api/event"
import { createSignal, onCleanup } from "solid-js"
import Loading from "../Components/Loading"
import UploadFiles from "./UploadFiles"
import WaitForReceiver from "./WaitForReceiver"
import TransferFiles from "./TransferFiles"

import { sendNotification } from "@tauri-apps/plugin-notification"
import { setStore, store } from "../App"
import { getFileNameFromPath } from "../utils"
import { FileUploadCardData } from "../Components/FileUploadCard"
import toast from "solid-toast"
import {
    CONNECTED_WITH_CONN_TYPE,
    FILES_DECISION_EVENT,
    TICKET_EVENT,
} from "../events"

export enum SendState {
    ChooseFiles = "S_choose-files",
    ConnectingToServer = "S_connecting-to-server",
    WaitingForReceiver = "S_waiting-for-receiver",
    WaitingForFileAccept = "S_waiting-for-file-accept",
    UploadingFiles = "S_uploading-files",
}

interface SendProps {
    files: string[]
    onError(error: string): void
}

function Send(props: SendProps) {
    const [code, setCode] = createSignal<string | null>(null)
    const [files, setFiles] = createSignal<FileUploadCardData[]>([])

    const unlisten1 = listen(TICKET_EVENT, (code: Event<string>) => {
        console.log("ticket event")
        setStore("currentState", SendState.WaitingForReceiver)
        setCode(code.payload)
    })

    const unlisten2 = listen(
        CONNECTED_WITH_CONN_TYPE,
        (_conn_type: Event<string>) => {
            setStore("currentState", SendState.WaitingForFileAccept)
        },
    )

    const unlisten3 = listen(
        FILES_DECISION_EVENT,
        (accepted: Event<boolean>) => {
            console.log(accepted)
            if (!accepted.payload) {
                toast.error("Files rejected")
                setStore("currentState", null)
            } else {
                setStore("currentState", SendState.UploadingFiles)
            }
        },
    )

    onCleanup(async () => {
        ;(await unlisten1)()
        ;(await unlisten2)()
        ;(await unlisten3)()
    })

    console.log(store.currentState)

    return (
        <div class="send">
            {store.currentState === SendState.ChooseFiles ? (
                <UploadFiles
                    initialFilePaths={props.files}
                    onSend={(fileData) => {
                        invoke("upload_files", {
                            files: fileData.map((file) => file.path),
                        }).catch((e: string) => {
                            props.onError(e)
                        })

                        setFiles(fileData)
                        setStore("currentState", SendState.ConnectingToServer)
                    }}
                />
            ) : store.currentState === SendState.ConnectingToServer ? (
                <Loading text="Connecting to server..." />
            ) : store.currentState === SendState.WaitingForReceiver ? (
                <WaitForReceiver code={code()!} />
            ) : store.currentState === SendState.WaitingForFileAccept ? (
                <Loading text="Waiting for receiver to accept files..." />
            ) : store.currentState === SendState.UploadingFiles ? (
                <TransferFiles
                    files={files().map((fileData) => [
                        getFileNameFromPath(fileData.path),
                        fileData.fileInfo.sizeBytes,
                        fileData.fileInfo.isDirectory,
                    ])}
                    type="send"
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

export default Send
