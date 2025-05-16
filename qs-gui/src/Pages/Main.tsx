import { TbFileUpload } from "solid-icons/tb"
import { onCleanup } from "solid-js"
import ReceiveCodeInput from "../Components/ReceiveCodeInput"
import { DragDropEvent } from "@tauri-apps/api/webview"
import { Event, listen } from "@tauri-apps/api/event"

interface MainProps {
    /// Callback for when the code is entered
    onEnterCode: (code: string) => void
    /// Callback for when files are initially dropped
    onFilesDropped: (paths: string[]) => void
}

function Main(props: MainProps) {
    /// Receiver

    const unlisten = listen(
        "tauri://drag-drop",
        (event: Event<DragDropEvent>) => {
            console.log("files dropped")
            const payload = event.payload
            const paths = (payload as any).paths // TODO: event has no type field (maybe bug on tauri?)
            props.onFilesDropped(paths)
        },
    )

    onCleanup(async () => {
        console.log("unlistening 1")
        ;(await unlisten)()
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
            <div class="text-center enter-code-text">
                <span>Enter code to receive</span>
                <div class="code-input">
                    <ReceiveCodeInput onSubmit={props.onEnterCode} />
                </div>
            </div>
            <footer class="version-footer">quic-send v0.4.0</footer>
        </div>
    )
}

export default Main
