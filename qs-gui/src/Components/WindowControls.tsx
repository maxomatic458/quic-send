import { window } from "@tauri-apps/api"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import {
    VsChromeClose,
    VsChromeMaximize,
    VsChromeMinimize,
    VsChromeRestore,
} from "solid-icons/vs"
import { createSignal } from "solid-js"

function WindowControls() {
    const [isMaximized, setIsMaximized] = createSignal(false)

    listen("tauri://resize", async () => {
        if (await window.getCurrentWindow().isMaximized()) {
            setIsMaximized(true)
        } else {
            setIsMaximized(false)
        }
    })

    function maximize() {
        if (isMaximized()) {
            window.getCurrentWindow().unmaximize()
        } else {
            window.getCurrentWindow().maximize()
        }
    }

    return (
        <div class="window-controls" data-tauri-drag-region>
            <div class="close-window-minimize window-control-button">
                <button onClick={() => window.getCurrentWindow().minimize()}>
                    <VsChromeMinimize />
                </button>
            </div>
            <div class="close-window-maximize window-control-button">
                <button onClick={maximize}>
                    {isMaximized() ? <VsChromeRestore /> : <VsChromeMaximize />}
                </button>
            </div>
            <div class="close-window-button window-control-button">
                <button onClick={() => invoke("exit", { code: 0 })}>
                    <VsChromeClose />
                </button>
            </div>
        </div>
    )
}

export default WindowControls
