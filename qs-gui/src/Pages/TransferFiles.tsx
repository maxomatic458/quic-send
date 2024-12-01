import { Event, listen } from "@tauri-apps/api/event"
import { createEffect, createSignal } from "solid-js"
import FileTransferCard from "../Components/FileTransferCard"
import { humanFileSize } from "../utils"
import { ProgressBarStatus, Window } from "@tauri-apps/api/window"
import { invoke } from "@tauri-apps/api/core"

const PROGRESS_CALL_INTERVAL_MS = 80

interface TransferFilesProps {
    type: "send" | "receive"
    /// name, size, isDir
    files: [string, number, boolean][]
    /// Callback for when the transfer is cancelled
    onCancel: () => void
    /// Callback for when the transfer is completed
    onComplete: () => void
}

interface TransferProgressEvent {
    ///name, downloaded, total
    data: [string, number, number][]
}

function TransferFiles(props: TransferFilesProps) {
    const [downloaded, setDownloaded] = createSignal<number>(0)
    const [totalSize, _setTotalSize] = createSignal<number>(
        props.files.reduce((acc, file) => acc + file[1], 0),
    )

    const [barProgress, setBarProgress] = createSignal<number[]>([])

    listen(
        "initial-download-progress",
        (event: Event<TransferProgressEvent>) => {
            let data = event.payload.data
            let downloaded = data.reduce((acc, file) => acc + file[1], 0)
            setDownloaded(downloaded)
        },
    )

    listen("transfer-complete", (_) => {
        // in case stuff gets out of sync somehow
        setDownloaded(totalSize())
    })

    setInterval(async () => {
        let downloaded: number = await invoke("bytes_transferred")
        setDownloaded(downloaded)
    }, PROGRESS_CALL_INTERVAL_MS)

    createEffect(async () => {
        let bytesLeft = downloaded()

        let progress: number[] = props.files.map((file) => {
            let size = file[1]
            let progress = 0
            if (bytesLeft >= size) {
                progress = size
                bytesLeft -= size
            } else {
                progress = bytesLeft
                bytesLeft = 0
            }

            return progress
        })

        setBarProgress(progress)

        const progressPercent = (downloaded() / totalSize()) * 100
        Window.getCurrent().setProgressBar({
            status: ProgressBarStatus.Normal,
            progress: Math.floor(progressPercent),
        })

        if (downloaded() == totalSize()) {
            props.onComplete()

            Window.getCurrent().setProgressBar({
                status: ProgressBarStatus.None,
            })
        }
    })

    return (
        <div class="transfer-files full-height">
            <h3 class="text-center" style={{ "margin-top": "2rem" }}>
                {props.type == "send" ? "Sending files" : "Receiving files"}
            </h3>
            <div class="file-list">
                {props.files.map((file, index) => {
                    return (
                        <FileTransferCard
                            progressBytes={barProgress()[index]}
                            sizeBytes={file[1]}
                            name={file[0]}
                            isDirectory={file[2]}
                        />
                    )
                })}
                <div class="file-size-all">
                    <span class="file-size-all-size">
                        {humanFileSize(downloaded(), true, 2)}
                    </span>
                    <span class="file-size-all-text">/</span>
                    <span class="file-size-all-size">
                        {humanFileSize(totalSize(), true, 2)}
                    </span>
                    <span class="file-size-all-text">Transferred</span>
                </div>
            </div>
            <div class="cancel-div">
                <button
                    class="file-choice-button file-choice-reject"
                    onClick={() => {
                        Window.getCurrent().setProgressBar({
                            status: ProgressBarStatus.None,
                        })
                        props.onCancel()
                    }}
                >
                    Cancel
                </button>
            </div>
        </div>
    )
}

export default TransferFiles
