import { createSignal, onMount } from "solid-js";

interface ReceiveCodeInputProps {
    /// Length of the receive code
    length: number
    /// Callback for when the code changes
    onChange: (code: string) => void
}

function ReceiveCodeInput(props: ReceiveCodeInputProps) {
    const [code, setCode] = createSignal<string>("");

    let inputs: HTMLDivElement;

    onMount(() => {
        if (inputs) {
            inputs.addEventListener("input", (e) => {
                const target = e.target as HTMLInputElement;
                const val = target.value;
                if (val != "") {
                    const next = target.nextElementSibling as HTMLInputElement;
                    if (next) {
                        next.focus();
                    }
                }
            })

            inputs.addEventListener("keyup", (e) => {
                const target = e.target as HTMLInputElement;
                const key = e.key;

                if (key == "Backspace" || key == "Delete") {
                    let prev = target.previousElementSibling as HTMLInputElement;
                    if (target.value.length == 0) {
                        console.log(target.value);
                        // prev = prev.previousElementSibling as HTMLInputElement;
                        // prev.value = "";
                    }

                    target.value = "";
                    if (prev) {
                        prev.focus();
                    }
                }

                return
            })
        }
    })

    return <div class="receive-code-input-container">
        <div class="inputs" id="inputs" ref={(el) => inputs = el}>
            {
                Array.from({ length: props.length }).map((_, i) => {
                    return <input
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