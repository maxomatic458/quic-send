import { AiOutlineFile, AiOutlineFolder } from "solid-icons/ai"
import { humanDuration, humanFileSize } from "../utils"
import { createMemo } from "solid-js"

interface FileCardProps {
    progressBytes?: number
    sizeBytes: number
    name: string
    isDirectory: boolean
    currentSpeedBps: number
}

function FileTransferCard(props: FileCardProps) {
    const progressColor = getComputedStyle(
        document.documentElement,
    ).getPropertyValue("--loading-bar-color")

    const background = createMemo(() => {
        const progressPercent =
            ((props.progressBytes ?? 0) / props.sizeBytes) * 100
        if (props.progressBytes) {
            return `linear-gradient(to right, ${progressColor} ${progressPercent}%, transparent ${progressPercent}%)`
        } else {
            return "transparent"
        }
    })

    // use the progressBytes and sizebytes and currentSpeedBps to determine the remaining time in seconds
    const humanRemainingTime = createMemo(() => {
        if (props.currentSpeedBps === 0) {
            return "?s"
        }
        const remainingBytes = props.sizeBytes - (props.progressBytes ?? 0)
        const remainingSeconds = Math.ceil(
            remainingBytes / props.currentSpeedBps,
        )

        return humanDuration(remainingSeconds, true)
    })

    return (
        <div
            class="file-card file-list-item"
            style={{ background: background() }}
        >
            <div class="file-list-item-icon">
                {props.isDirectory ? (
                    <AiOutlineFolder size={"1.4rem"} />
                ) : (
                    <AiOutlineFile size={"1.4rem"} />
                )}
            </div>
            <div class="file-list-item-text">
                <div class="file-list-item-name">{props.name}</div>
                {props.progressBytes ? (
                    <div class="file-list-item-progress">
                        <span class="file-list-item-progress">
                            {humanFileSize(props.progressBytes, true, 2)}
                        </span>
                        <span> / </span>
                        <span class="file-list-item-size">
                            {humanFileSize(props.sizeBytes, true, 2)}
                        </span>
                        <span class="file-list-item-remaining">
                            {humanRemainingTime()}
                        </span>
                    </div>
                ) : (
                    <div class="file-list-item-size">
                        {humanFileSize(props.sizeBytes, true, 2)}
                    </div>
                )}
            </div>
        </div>
    )
}

export default FileTransferCard
