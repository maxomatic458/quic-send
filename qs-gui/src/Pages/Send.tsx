import { invoke } from "@tauri-apps/api/core"
import { Event, listen } from "@tauri-apps/api/event"
import { createSignal } from "solid-js"
import Loading from "../Components/Loading"
import UploadFiles from "./UploadFiles"
import WaitForReceiver from "./WaitForReceiver"
import TransferFiles from "./TransferFiles"

import {
    isPermissionGranted,
    requestPermission,
    sendNotification,
} from "@tauri-apps/plugin-notification"
import { getFileNameFromPath } from "../utils"
import { store } from "../App"

type SendState =
    | "choose-files"
    | "connecting-to-server"
    | "waiting-for-receiver"
    | "waiting-for-file-accept"
    | "uploading-files"

interface ReceiveProps {
    files: string[]
    onError(error: string): void
    onCancel(): void
}

function Send(props: ReceiveProps) {
    const [code, setCode] = createSignal<string | null>(null)
    const [state, setState] = createSignal<SendState>("choose-files")
    const [files, setFiles] = createSignal<[string, number, boolean][]>([])

    listen("server-connection-code", (code: Event<string>) => {
        setState("waiting-for-receiver")
        setCode(code.payload)
    })

    listen("receiver-connected", (_) => {
        console.log("receiver connected")
        setState("waiting-for-file-accept")
        console.log("receiver connected")
    })

    listen("files-decision", (accepted) => {
        if (!accepted) {
            props.onError("Receiver declined files")
            return
        }
        setState("uploading-files")
        console.log("files accepted")
    })

    return (
        <div class="send">
            {state() == "choose-files" ? (
                <UploadFiles
                    files={props.files}
                    onAddFiles={(newFiles) =>
                        setFiles([...files(), ...newFiles])
                    }
                    onRemoveFile={(idx) =>
                        setFiles(files().filter((_, i) => i != idx))
                    }
                    onSend={() => {
                        invoke("upload_files", {
                            serverAddr: store.roundezvousAddr,
                            files: files().map(([path, _, __]) => path),
                        }).catch((e: string) => {
                            props.onError(e)
                        })

                        setState("connecting-to-server")
                    }}
                    onCancel={() => props.onCancel()}
                />
            ) : state() == "connecting-to-server" ? (
                <Loading text="Connecting to server..." />
            ) : state() == "waiting-for-receiver" ? (
                <WaitForReceiver code={code()!} />
            ) : state() == "waiting-for-file-accept" ? (
                <Loading text="Waiting for receiver to accept files..." />
            ) : state() == "uploading-files" ? (
                <TransferFiles
                    files={files()}
                    type="send"
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

export default Send
