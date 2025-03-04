import { createEffect, createSignal } from "solid-js"
import "./App.css"
import { createStore } from "solid-js/store"
import Main from "./Pages/Main"
import Receive, { ReceiveState } from "./Pages/Receive"
import toast, { Toaster } from "solid-toast"
import Send, { SendState } from "./Pages/Send"
import WindowControls from "./Components/WindowControls"
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window"

export interface AppState {
    currentState: CurrentState
}

export const [store, setStore] = createStore<AppState>({
    currentState: null,
})

type CurrentState = ReceiveState | SendState | null

function isRecvState(state: CurrentState): boolean {
    return (
        state === ReceiveState.ConnectingToServer ||
        state === ReceiveState.ConnectingToSender ||
        state === ReceiveState.WaitingForFiles ||
        state === ReceiveState.FilesOffered ||
        state === ReceiveState.DownloadingFiles
    )
}

function App() {
    /// Receiver
    const [code, setCode] = createSignal<string | null>(null)

    /// Sender
    const [files, setFiles] = createSignal<string[]>([])

    function handleError(e: string) {
        toast.error(e)
        console.error(e)
        setStore("currentState", null)
        getCurrentWindow().setProgressBar({
            status: ProgressBarStatus.None,
            progress: 0,
        })
    }

    function disableRefresh() {
        document.addEventListener("keydown", function (event) {
            // Prevent F5 or Ctrl+R (Windows/Linux) and Command+R (Mac) from refreshing the page
            if (
                event.key === "F5" ||
                (event.ctrlKey && event.key === "r") ||
                (event.metaKey && event.key === "r")
            ) {
                event.preventDefault()
            }
        })

        document.addEventListener("contextmenu", function (event) {
            event.preventDefault()
        })
    }

    disableRefresh()

    createEffect(() => {
        if (store.currentState === null) {
            console.log("ok")
            setCode(null)
            setFiles([])
        }
    })

    return (
        <>
            <Toaster position="top-right" gutter={8} />
            <div class="app">
                <WindowControls />

                {store.currentState === null ? (
                    <Main
                        onEnterCode={(code) => {
                            setCode(code)
                            setStore(
                                "currentState",
                                ReceiveState.ConnectingToServer,
                            )
                        }}
                        onFilesDropped={(paths) => {
                            setFiles(paths)
                            setStore("currentState", SendState.ChooseFiles)
                        }}
                    />
                ) : isRecvState(store.currentState) ? (
                    <Receive code={code() as string} onError={handleError} />
                ) : (
                    <Send files={files()} onError={handleError} />
                )}
            </div>
        </>
    )
}

export default App
