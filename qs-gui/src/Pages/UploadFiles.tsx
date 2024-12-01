import { Event, listen } from "@tauri-apps/api/event"
import { DragDropEvent } from "@tauri-apps/api/webviewWindow"
import { createEffect, createSignal } from "solid-js"
import FileUploadCard, {
    FileInfo,
    FileUploadCardData,
} from "../Components/FileUploadCard"
import { invoke } from "@tauri-apps/api/core"
import toast from "solid-toast"
import { humanFileSize } from "../utils"

interface UploadFilesProps {
    files: string[]
    onAddFiles(files: [string, number, boolean][]): void
    onRemoveFile(idx: number): void
    onSend(): void
    onCancel(): void
}

function UploadFiles(props: UploadFilesProps) {
    // FIXME: refactor because fallback is not used
    const [files, setFiles] = createSignal<FileUploadCardData[]>([])
    const [cancel, setCancel] = createSignal(false)
    const processedFiles = new Set<string>()

    const fetchFileInfo = async (path: string) => {
        const fileInfo = await invoke<FileInfo>("file_info", { path })
        return fileInfo
    }

    const handleNewFiles = async (newFiles: string[]) => {
        const newFileData = await Promise.all(
            newFiles.map(async (file) => {
                if (!processedFiles.has(file)) {
                    processedFiles.add(file)
                    const fileInfo = await fetchFileInfo(file)
                    return { path: file, fileInfo }
                } else {
                    toast.error("File already added")
                }
                return null
            }),
        )

        props.onAddFiles(
            newFileData
                .filter((file) => file !== null)
                .map((file) => [
                    file.path,
                    file.fileInfo.sizeBytes,
                    file.fileInfo.isDirectory,
                ]),
        )

        setFiles([...files(), ...newFileData.filter((file) => file !== null)])
    }

    handleNewFiles(props.files)

    listen("tauri://drag-drop", (event: Event<DragDropEvent>) => {
        const payload = event.payload
        const paths: string[] = (payload as any).paths // TODO: event has no type field (maybe bug on tauri?)
        handleNewFiles(paths)
    })

    createEffect(() => {
        if (cancel()) {
            props.onCancel()
            setCancel(false)
        }
    })

    return (
        <div class="upload-files full-height">
            <h3 class="text-center" style={{ "margin-top": "2rem" }}>
                Files to send
            </h3>
            <div class="file-list">
                {files().map((file, idx) => (
                    <FileUploadCard
                        data={file}
                        id={idx.toString()}
                        onRemove={() => {
                            props.onRemoveFile(idx)
                            setFiles(files().filter((_, i) => i != idx))
                            processedFiles.delete(file.path)

                            if (files().length == 0) {
                                setCancel(true)
                            }
                        }}
                    />
                ))}
                <div class="file-size-all">
                    <span class="file-size-all-text">Total size</span>
                    <span class="file-size-all-size">
                        {humanFileSize(
                            files().reduce(
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
                        if (props.files.length > 0) props.onSend()
                    }}
                    disabled={props.files.length == 0}
                >
                    Send
                </button>
            </div>
        </div>
    )
}

export default UploadFiles
