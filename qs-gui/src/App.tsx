import { createSignal } from "solid-js"
import "./App.css"
import { createStore } from "solid-js/store"
import Main from "./Pages/Main"
import Receive from "./Pages/Receive"
import toast, { Toaster } from "solid-toast"
import Send from "./Pages/Send"
import WaitForReceiver from "./Pages/WaitForReceiver"

export interface AppState {
    roundezvousAddr: string
}

export const [store, setStore] = createStore<AppState>({
    roundezvousAddr: "209.25.141.16:1172",
})

function App() {
    /// Receiver
    const [code, setCode] = createSignal<string | null>(null)

    /// Sender
    const [files, setFiles] = createSignal<string[]>([])

    function handleError(e: string) {
        toast.error(e)
        console.error(e)
        setCode(null)
        setFiles([])
    }

    function cancel() {
        setCode(null)
        setFiles([])
    }

    const disableRefresh = () => {
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

    return (
        <>
            <Toaster position="top-right" gutter={8} />
            <div class="app">
                {code() !== null ? (
                    <Receive code={code() as string} onError={handleError} />
                ) : files().length > 0 ? (
                    <Send
                        files={files()}
                        onError={handleError}
                        onCancel={cancel}
                    />
                ) : (
                    <Main
                        onEnterCode={(code) => setCode(code)}
                        onFilesDropped={(files) => setFiles(files)}
                    />
                )}
            </div>
        </>
        // <WaitForReceiver code="12344678" />
    )
}

export default App
