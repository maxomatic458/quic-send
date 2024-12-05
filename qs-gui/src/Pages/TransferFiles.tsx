import { Event, listen } from "@tauri-apps/api/event"
import { createEffect, createSignal, on, onCleanup } from "solid-js"
import FileTransferCard from "../Components/FileTransferCard"
import { humanFileSize } from "../utils"
import { ProgressBarStatus, Window } from "@tauri-apps/api/window"
import { invoke } from "@tauri-apps/api/core"
import { setStore } from "../App"

const PROGRESS_CALL_INTERVAL_MS = 80

interface TransferFilesProps {
    type: "send" | "receive"
    /// name, size, isDir
    files: [string, number, boolean][]
    /// Callback for when the transfer is completed
    onComplete: () => void
}

interface TransferProgressEvent {
    ///name, downloaded, total
    data: [string, number, number][]
}

function TransferFiles(props: TransferFilesProps) {
    const [initialProgress, setInitialProgress] = createSignal<number>(0)
    const [downloaded, setDownloaded] = createSignal<number>(0)
    const [totalSize, _setTotalSize] = createSignal<number>(
        props.files.reduce((acc, file) => acc + file[1], 0),
    )

    /// The number is the number of bytes downloaded for each file
    const [barProgress, setBarProgress] = createSignal<number[]>([])
    const [initialBarProgress, setInitialBarProgress] = createSignal<number[]>(
        [],
    )

    const unlisten1 = listen(
        "initial-progress",
        (event: Event<TransferProgressEvent>) => {
            console.log("initial progress")
            let data = event.payload.data
            setInitialProgress(
                data.reduce((acc, file) => acc + file[1], 0) as number,
            )

            console.log(data)
            setInitialBarProgress(data.map((file) => file[1]))
        },
    )

    const unlisten2 = listen("transfer-done", (_) => {
        setDownloaded(totalSize() - initialProgress())
    })

    onCleanup(async () => {
        ;(await unlisten1)()
        ;(await unlisten2)()
    })

    setInterval(async () => {
        let downloaded: number = await invoke("bytes_transferred")
        setDownloaded(downloaded)
        console.log(downloaded)
    }, PROGRESS_CALL_INTERVAL_MS)

    createEffect(
        on(downloaded, async () => {
            let bytesDownloadedAll = downloaded() + initialProgress()

            let leftToAdd = downloaded()

            let progress: number[] = initialBarProgress().map(
                (initialBytesDownloaded, index) => {
                    let currentProgress = initialBytesDownloaded
                    if (leftToAdd > 0) {
                        let toAdd = Math.min(
                            leftToAdd,
                            props.files[index][1] - currentProgress,
                        )
                        leftToAdd -= toAdd
                        currentProgress += toAdd
                    }
                    return currentProgress
                },
            )

            setBarProgress(progress)

            const progressPercent = (bytesDownloadedAll / totalSize()) * 100
            Window.getCurrent().setProgressBar({
                status: ProgressBarStatus.Normal,
                progress: Math.floor(progressPercent),
            })

            console.log("downloaded", bytesDownloadedAll)
            console.log("total", totalSize())

            if (bytesDownloadedAll == totalSize()) {
                props.onComplete()

                Window.getCurrent().setProgressBar({
                    status: ProgressBarStatus.None,
                })
            }
        }),
    )

    return (
        <div class="transfer-files">
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
                        {humanFileSize(
                            downloaded() + initialProgress(),
                            true,
                            2,
                        )}
                    </span>
                    <span class="file-size-all-text">/</span>
                    <span
                        class="file-size-all-size"
                        style={{ "margin-right": "0.5rem" }}
                    >
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
                        invoke("exit", { code: 0 })
                    }}
                >
                    Cancel
                </button>
            </div>
        </div>
    )
}

export default TransferFiles
