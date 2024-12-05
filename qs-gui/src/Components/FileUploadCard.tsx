import { AiOutlineFile, AiOutlineFolder } from "solid-icons/ai"
import { getFileNameFromPath, humanFileSize } from "../utils"
import { ImCancelCircle } from "solid-icons/im"

interface FileUploadCardProps {
    id: string
    data: FileUploadCardData
    onRemove(): void
}

export interface FileUploadCardData {
    path: string
    fileInfo: FileInfo
}

export interface FileInfo {
    sizeBytes: number
    isDirectory: boolean
}

function FileUploadCard(props: FileUploadCardProps) {
    return (
        <div class="file-list-item">
            <div class="file-list-item-icon">
                {props.data.fileInfo.isDirectory ? (
                    <AiOutlineFolder size={"1.4rem"} />
                ) : (
                    <AiOutlineFile size={"1.4rem"} />
                )}
            </div>
            <div class="file-list-item-text">
                <div class="file-list-item-name">
                    {getFileNameFromPath(props.data.path)}
                </div>
                <div class="flex flex-row">
                    <span
                        class="file-list-item-size"
                        style={{ "margin-right": "1rem" }}
                    >
                        {humanFileSize(props.data.fileInfo.sizeBytes, true, 2)}
                    </span>
                    <span class="remove-file-upload">
                        <button onClick={props.onRemove}>
                            <ImCancelCircle />
                        </button>
                    </span>
                </div>
            </div>
        </div>
    )
}

export default FileUploadCard
