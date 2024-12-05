import { invoke } from "@tauri-apps/api/core"
import { TbFileUpload } from "solid-icons/tb"
import { createEffect, createSignal } from "solid-js"
import ReceiveCodeInput from "../Components/ReceiveCodeInput"
import { DragDropEvent } from "@tauri-apps/api/webview"
import { Event, listen } from "@tauri-apps/api/event"
import { store } from "../App"

interface MainProps {
    /// Callback for when the code is entered
    onEnterCode: (code: string) => void
    /// Callback for when files are initially dropped
    onFilesDropped: (paths: string[]) => void
}

function Main(props: MainProps) {
    /// Receiver
    const [code, setCode] = createSignal<string>("")
    const [codeLength, setCodeLength] = createSignal<number | null>(null)

    invoke("code_len", {}).then((res) => {
        setCodeLength(res as number)
    })

    const unlisten = listen(
        "tauri://drag-drop",
        (event: Event<DragDropEvent>) => {
            const payload = event.payload
            const paths = (payload as any).paths // TODO: event has no type field (maybe bug on tauri?)
            props.onFilesDropped(paths)
        },
    )

    createEffect(async () => {
        if (store.currentState !== null) {
            ;(await unlisten)()
        }

        if (code().length === codeLength()!) {
            props.onEnterCode(code())
        }
    })

    return (
        <div class="flex flex-col">
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
            <div class="text-center enter-code-text">
                <span>Enter code to receive</span>
                <div class="code-input">
                    <ReceiveCodeInput
                        length={codeLength()!}
                        onChange={setCode}
                    />
                </div>
            </div>
            <footer class="version-footer">quic-send v0.3.0</footer>
        </div>
    )
}

export default Main
