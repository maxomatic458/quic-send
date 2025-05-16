import { createSignal } from "solid-js"
import { humanFileSize } from "../utils"
import FileTransferCard from "../Components/FileTransferCard"
import { open } from "@tauri-apps/plugin-dialog"

interface AcceptFilesProps {
    files: [string, number, boolean][]
    acceptFiles: (path: string | null) => void
}

function AcceptFiles(props: AcceptFilesProps) {
    const [downloadPath, setDownloadPath] = createSignal<string | null>(null)

    return (
        <div class="accept-files">
            <h3 class="text-center" style={{ "margin-top": "2rem" }}>
                Files offered
            </h3>
            <div class="file-list">
                {props.files.map((file) => {
                    return (
                        <FileTransferCard
                            sizeBytes={file[1]}
                            name={file[0]}
                            isDirectory={file[2]}
                            currentSpeedBps={0}
                        />
                    )
                })}
                <div class="file-size-all">
                    <span class="file-size-all-text">Total size</span>
                    <span class="file-size-all-size">
                        {humanFileSize(
                            props.files.reduce((acc, file) => acc + file[1], 0),
                            true,
                            2,
                        )}
                    </span>
                </div>
                <div class="select-download-path">
                    <button
                        onClick={() => {
                            open({
                                directory: true,
                                multiple: false,
                            }).then((res) => {
                                if (res) {
                                    setDownloadPath(res)
                                }
                            })
                        }}
                    >
                        {downloadPath() ?? "Select download path"}
                    </button>
                </div>
            </div>
            <div class="file-choice">
                <button
                    class="file-choice-button file-choice-reject"
                    onClick={() => props.acceptFiles(null)}
                >
                    Reject
                </button>
                <button
                    class="file-choice-button file-choice-accept"
                    onClick={() => {
                        if (downloadPath() != null)
                            props.acceptFiles(downloadPath())
                    }}
                    disabled={downloadPath() == null}
                >
                    Accept
                </button>
            </div>
        </div>
    )
}

export default AcceptFiles
