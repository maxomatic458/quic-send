import { invoke } from "@tauri-apps/api/tauri";
import { Accessor, createEffect, createSignal, Setter } from "solid-js";
import { AiFillFile } from "solid-icons/ai";
import { AiFillFolder } from "solid-icons/ai";
import humanFormat from "human-format";
import { FileDropEvent } from "@tauri-apps/api/window";
import { Event, listen } from "@tauri-apps/api/event";
import { FaSolidTrashCan } from "solid-icons/fa";

function getFileName(filePath: string) {
    return filePath.split(/[/\\]/).pop();
}
type FileToSendProps = {
    idx: number;
    filePath: string;
    // Only accurate for files, not directories
    // TODO: maybe calculate the FilesAvailable before sending also in rust maybe?
    sizeBytes: number;
    isDirectory: boolean;
    deleteCallback: () => void;
};

function FileToSend(props: FileToSendProps) {
    const fileName = getFileName(props.filePath);

    return (
        <div class="file-to-send">
            <div class="file-to-send-icon">
                {props.isDirectory ? <AiFillFolder /> : <AiFillFile />}
            </div>
            <div class="file-to-send-name">{fileName}</div>
            <div class="file-to-send-delete" onClick={props.deleteCallback}>
                <FaSolidTrashCan />
            </div>
        </div>
    );
}

type FilesToSendListProps = {
    initialFilePaths: string[];
};

function FilesToSendList(props: FilesToSendListProps) {
    const [files, setFiles] = createSignal<FileToSendProps[]>([]);
    const [filesLoadedOnce, setFilesLoadedOnce] = createSignal<boolean>(false);
    let [allFilesIndex, setAllFilesIndex] = createSignal<number>(0);

    props.initialFilePaths.forEach((filePath) => {
        setAllFilesIndex((prev) => prev + 1);
        const currentIdx = allFilesIndex();
        invoke("get_file_size_and_is_dir", { path: filePath })
            .then((result) => {
                let [size, isDir]: [number, boolean] = result as [
                    number,
                    boolean,
                ];
                setFiles((prev) => [
                    ...prev,
                    {
                        idx: currentIdx,
                        filePath: filePath,
                        sizeBytes: size,
                        isDirectory: isDir,
                        deleteCallback: () => {
                            setFiles((prev) =>
                                prev.filter((file) => file.idx != currentIdx),
                            );
                        },
                    },
                ]);
                setFilesLoadedOnce(true);
            })
            .catch((err) => {
                console.error(err);
            });
    });

    listen("tauri://file-drop", async (event: Event<FileDropEvent>) => {
        (event.payload as any as string[]).forEach((filePath) => {
            setAllFilesIndex((prev) => prev + 1);
            const currentIdx = allFilesIndex();
            console.log(filePath);
            console.log("SETTING: ", allFilesIndex());
            invoke("get_file_size_and_is_dir", { path: filePath })
                .then((result) => {
                    const [size, isDir]: [number, boolean] = result as [
                        number,
                        boolean,
                    ];
                    const file = {
                        idx: currentIdx,
                        filePath: filePath,
                        sizeBytes: size,
                        isDirectory: isDir,
                        deleteCallback: () => {
                            setFiles((prev) =>
                                prev.filter((file) => file.idx != currentIdx),
                            );
                        },
                    };

                    setFiles((prev) => [...prev, file]);
                })
                .catch((err) => {
                    console.error(err);
                });
        });
    });

    createEffect(() => {
        // if all files removed go back
        if (files().length == 0 && filesLoadedOnce()) {
            window.location.href = "/";
        }
    });

    return (
        <div>
            <div class="files-to-send-list-inner">
                {files().map((file) => (
                    <FileToSend {...file} />
                ))}
            </div>
        </div>
    );
}

export default FilesToSendList;
