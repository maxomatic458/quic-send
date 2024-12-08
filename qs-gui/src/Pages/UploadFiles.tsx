import { Event, listen } from "@tauri-apps/api/event"
import { DragDropEvent } from "@tauri-apps/api/webviewWindow"
import { createSignal, onCleanup, onMount } from "solid-js"
import FileUploadCard, {
    FileInfo,
    FileUploadCardData,
} from "../Components/FileUploadCard"
import { invoke } from "@tauri-apps/api/core"
import toast from "solid-toast"
import { getFileNameFromPath, humanFileSize } from "../utils"
import { setStore } from "../App"

interface UploadFilesProps {
    initialFilePaths: string[]
    onSend(filesToSend: FileUploadCardData[]): void
}

function UploadFiles(props: UploadFilesProps) {
    const [files, setFiles] = createSignal<Map<string, FileUploadCardData>>(
        new Map(),
    )

    async function fileInfo(path: string): Promise<FileInfo> {
        const fileInfo = await invoke<FileInfo>("file_info", { path })
        return fileInfo
    }

    async function handleNewFiles(newFiles: string[]) {
        let newFilesMap: Map<string, FileUploadCardData> = new Map()

        console.log(newFiles)

        for (const path of newFiles) {
            if (files().has(path)) {
                console.log(files())
                const fileName = getFileNameFromPath(path)
                toast.error(`${fileName} already added`)
                continue
            }

            try {
                const info = await fileInfo(path)
                newFilesMap.set(path, { path, fileInfo: info })
            } catch (error) {
                console.error(`Failed to get file info for ${path}`, error)
            }
        }

        if (newFilesMap.size === 0) {
            return
        }

        let updatedFiles = new Map(files())
        newFilesMap.forEach((value, key) => updatedFiles.set(key, value))
        setFiles(updatedFiles)
    }

    onMount(() => {
        handleNewFiles(props.initialFilePaths)
    })

    const unlisten = listen(
        "tauri://drag-drop",
        (event: Event<DragDropEvent>) => {
            const payload = event.payload
            const paths: string[] = (payload as any).paths // TODO: event has no type field (maybe bug on tauri?)
            handleNewFiles(paths)
        },
    )

    onCleanup(async () => {
        ;(await unlisten)()
    })

    return (
        <div class="upload-files">
            <h3 class="text-center" style={{ "margin-top": "2rem" }}>
                Files to send
            </h3>
            <div class="file-list">
                {Array.from(files().values()).map((file, index) => (
                    <FileUploadCard
                        data={file}
                        id={index.toString()}
                        onRemove={() => {
                            const newFiles = new Map(files())
                            newFiles.delete(file.path)
                            setFiles(newFiles)

                            if (newFiles.size === 0) {
                                setStore("currentState", null)
                                console.log("No files to send")
                            }
                        }}
                    />
                ))}

                <div class="file-size-all">
                    <span class="file-size-all-text">Total size</span>
                    <span class="file-size-all-size">
                        {humanFileSize(
                            Array.from(files().values()).reduce(
                                (acc, file) => acc + file.fileInfo.sizeBytes,
                                0,
                            ),
                            true,
                            2,
                        )}
                    </span>
                </div>
            </div>
            <div class="send-div">
                <button
                    class="file-choice-button file-choice-accept"
                    onClick={() => {
                        if (files().size > 0) {
                            props.onSend(Array.from(files().values()))
                        }
                    }}
                    disabled={files().size === 0}
                >
                    Send
                </button>
            </div>
        </div>
    )
}

export default UploadFiles
