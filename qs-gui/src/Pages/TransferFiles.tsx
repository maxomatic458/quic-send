import { Event, listen } from "@tauri-apps/api/event"
import { createEffect, createSignal, on, onCleanup } from "solid-js"
import FileTransferCard from "../Components/FileTransferCard"
import { humanDuration, humanFileSize } from "../utils"
import { ProgressBarStatus, Window } from "@tauri-apps/api/window"
import { invoke } from "@tauri-apps/api/core"
import { setStore } from "../App"
import {
    CANCEL_TRANSFER_EVENT,
    INITIAL_PROGRESS_EVENT,
    TRANSFER_FINISHED_EVENT,
} from "../events"

const SPEED_HISTORY_WINDOW_MS = 30_000
const ETA_CALCULATION_INTERVAL_MS = 1000
const PROGRESS_CALL_INTERVAL_MS = 80

interface TransferFilesProps {
    type: "send" | "receive"
    /// name, size, isDir
    files: [string, number, boolean][]
    /// Callback for when the transfer is completed
    onComplete: () => void
    /// The transfer mode
    transferMode: "direct" | "mixed" | "relay" | null
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

    const speedHistory: [number, number][] = []
    const [transferSpeedBps, setTransferSpeedBps] = createSignal<number>(0)
    const [totalRemainingSecs, setTotalRemainingSecs] = createSignal<number>(0)

    const speedIntervalId = setInterval(() => {
        const now = Date.now()
        const nowDownloaded = downloaded()

        // Add current point to history
        speedHistory.push([now, nowDownloaded])

        // Remove points older than SPEED_HISTORY_WINDOW_MS
        while (
            speedHistory.length > 1 &&
            now - speedHistory[0][0] > SPEED_HISTORY_WINDOW_MS
        ) {
            speedHistory.shift()
        }

        // Calculate average speed over the window
        if (speedHistory.length > 1) {
            const [oldestTime, oldestDownloaded] = speedHistory[0]
            const [latestTime, latestDownloaded] =
                speedHistory[speedHistory.length - 1]
            const timeDelta = (latestTime - oldestTime) / 1000 // seconds
            const bytesDelta = latestDownloaded - oldestDownloaded
            const speed = timeDelta > 0 ? bytesDelta / timeDelta : 0
            setTransferSpeedBps(speed)

            const remainingBytes =
                totalSize() - (nowDownloaded + initialProgress())
            const remainingSecs = Math.ceil(remainingBytes / speed)
            setTotalRemainingSecs(remainingSecs)
        } else {
            setTransferSpeedBps(0)
            setTotalRemainingSecs(0)
        }
    }, ETA_CALCULATION_INTERVAL_MS)

    const unlisten1 = listen(
        INITIAL_PROGRESS_EVENT,
        (event: Event<TransferProgressEvent>) => {
            console.log("Got initial progress")
            let data = event.payload.data
            setInitialProgress(
                data.reduce((acc, file) => acc + file[1], 0) as number,
            )

            setInitialBarProgress(data.map((file) => file[1]))
        },
    )

    const unlisten2 = listen(TRANSFER_FINISHED_EVENT, (_) => {
        setDownloaded(totalSize() - initialProgress())
    })

    const progressUpdaterId = setInterval(async () => {
        let downloaded: number = await invoke("bytes_transferred")
        setDownloaded(downloaded)
    }, PROGRESS_CALL_INTERVAL_MS)

    onCleanup(async () => {
        console.log("Start transfer cleanup")
        ;(await unlisten1)()
        ;(await unlisten2)()

        clearInterval(progressUpdaterId)
        clearInterval(speedIntervalId)
        console.log("End transfer cleanup")
    })

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
                            currentSpeedBps={transferSpeedBps()}
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
                <div class="total-remaining-time">
                    <span class="total-remaining-time-text">
                        Time remaining:
                    </span>
                    <span class="total-remaining-time-value">
                        {humanDuration(totalRemainingSecs(), false)}
                    </span>
                </div>
            </div>
            <div class="cancel-div">
                <button
                    class="file-choice-button file-choice-reject"
                    onClick={() => {
                        Window.getCurrent().setProgressBar({
                            status: ProgressBarStatus.None,
                        })
                        Window.getCurrent().emit(CANCEL_TRANSFER_EVENT, null)
                        setStore("currentState", null)
                        console.log("cancel transfer")
                    }}
                >
                    Cancel
                </button>

                <div class="transfer-mode">
                    <span class="transfer-mode-text">Mode:</span>
                    <span
                        class={`transfer-mode-common transfer-mode-${props.transferMode}`}
                    >
                        {props.transferMode ?? "???"}
                    </span>
                </div>
            </div>
        </div>
    )
}

export default TransferFiles
