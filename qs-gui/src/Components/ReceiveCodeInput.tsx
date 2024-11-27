import { createEffect, createSignal, onMount } from "solid-js";
import { redirect } from "@solidjs/router";

interface ReceiveCodeInputProps {
    /// Length of the receive code
    length: number
    /// Callback for when the code changes
    onChange: (code: string) => void
}

function ReceiveCodeInput(props: ReceiveCodeInputProps) {
    const [code, setCode] = createSignal<string>("");

    function onParentInput(e: InputEvent) {
        const target = e.target as HTMLInputElement;
        const val = target.value;
        if (val != "") {
            const next = target.nextElementSibling as HTMLInputElement;
            if (next) {
                next.focus();
            }
        }
    }

    function onParentKeyDown(e: KeyboardEvent) {
        const target = e.target as HTMLInputElement;
        const key = e.key;

        if (key == "Backspace" || key == "Delete") {
            let prev = target.previousElementSibling as HTMLInputElement;
            if (prev) {
                target.value = "";
                prev.focus();
            }
        }
    }

    return <div class="receive-code-input-container">
        <div
            class="inputs"
            id="inputs"
            onInput={onParentInput}
            onKeyDown={onParentKeyDown}
        >
            {
                Array.from({ length: props.length }).map((_, i) => {
                    return <input
                        maxlength={1}
                        type="text"
                        value={code()[i] ?? ""}
                        onInput={(e) => {
                            let newCode = code();
                            let newCodeArr = newCode.split("");
                            newCodeArr[i] = (e.target as HTMLInputElement).value;
                            newCode = newCodeArr.join("");
                            setCode(newCode);
                            props.onChange(newCode);
                        }}
                    />
                })
            }
        </div>
    </div>
}

export default ReceiveCodeInput;