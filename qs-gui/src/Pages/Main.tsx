import { invoke } from "@tauri-apps/api/core";
import { TbFileUpload } from "solid-icons/tb";
import { createSignal } from "solid-js";
import ReceiveCodeInput from "../Components/ReceiveCodeInput";

function Main() {
    const [code, setCode] = createSignal<string>("");
    const [codeLength, setCodeLength] = createSignal<number>(0);

    invoke("code_len", {}).then((res) => {
        setCodeLength(res as number);
    });

    return <div class="main flex flex-col">
        <div class="main-upload-area">
            <div>
                <div class="upload-icon" style={{"text-align": "center"}}>
                    <TbFileUpload size={"50px"}/>
                </div>
                <div class="upload-text">
                    Drop files to send
                </div>
            </div>
        </div>
        <div class="code-input">
            <ReceiveCodeInput length={6} onChange={setCode} />
        </div>
    </div>
}

export default Main;