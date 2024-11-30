import { invoke } from "@tauri-apps/api/core"
import { TbFileUpload } from "solid-icons/tb"
import { createEffect, createSignal } from "solid-js"
import ReceiveCodeInput from "../Components/ReceiveCodeInput"
import { DragDropEvent } from "@tauri-apps/api/webview"
import { Event, listen } from "@tauri-apps/api/event"

interface MainProps {
    /// Callback for when the code is entered
    onEnterCode: (code: string) => void
    /// Callback for when files are dropped
    onFilesDropped: (files: string[]) => void
}

function Main(props: MainProps) {
    /// Receiver
    const [code, setCode] = createSignal<string>("")
    const [codeLength, setCodeLength] = createSignal<number | null>(null)

    invoke("code_len", {}).then((res) => {
        setCodeLength(res as number)
    })

    listen("tauri://drag-drop", (event: Event<DragDropEvent>) => {
        const payload = event.payload
        const paths = (payload as any).paths // TODO: event has no type field (maybe bug on tauri?)
        props.onFilesDropped(paths)
    })

    createEffect(() => {
        if (code().length == codeLength()) {
            props.onEnterCode(code())
        }
    })

    return (
        <div class="main flex flex-col">
            <h2 class="text-center" style={{ "margin-top": "2rem" }}>
                quic send
            </h2>
            <div class="main-upload-area">
                <div>
                    <div class="upload-icon" style={{ "text-align": "center" }}>
                        <TbFileUpload size={"50px"} />
                    </div>
                    <div class="upload-text">Drop files to send</div>
                </div>
            </div>
            <div class="code-input">
                <ReceiveCodeInput
                    length={codeLength() ?? 0}
                    onChange={setCode}
                />
            </div>
            <footer class="text-center">quic-send v0.3.0</footer>
        </div>
    )
}

export default Main
