import { createSignal } from "solid-js"

interface ReceiveCodeInputProps {
    /// Length of the receive code
    length: number
    /// Callback for when the code changes
    onChange: (code: string) => void
}

function ReceiveCodeInput(props: ReceiveCodeInputProps) {
    const [code, setCode] = createSignal<string>("")

    function onParentInput(e: InputEvent) {
        const target = e.target as HTMLInputElement
        const val = target.value
        if (val != "") {
            const next = target.nextElementSibling as HTMLInputElement
            if (next) {
                next.focus()
            }
        }
    }

    function onParentKeyDown(e: KeyboardEvent) {
        const target = e.target as HTMLInputElement
        const key = e.key

        if (key == "Backspace" || key == "Delete") {
            let prev = target.previousElementSibling as HTMLInputElement
            if (prev) {
                target.value = ""
                prev.focus()
            }
        }
    }

    function onParentPaste(e: ClipboardEvent) {
        e.preventDefault()
        const pasteData = e.clipboardData?.getData("text") || ""
        const limitedPasteData = pasteData.slice(0, props.length)

        setCode(limitedPasteData)
        props.onChange(limitedPasteData)

        const inputs = Array.from(
            document.querySelectorAll(".receive-code-input-container input"),
        ) as HTMLInputElement[]

        limitedPasteData.split("").forEach((char, i) => {
            if (inputs[i]) {
                inputs[i].value = char
            }
        })

        const nextInput = inputs[limitedPasteData.length]
        if (nextInput) {
            nextInput.focus()
        }
    }

    return (
        <div class="receive-code-input-container">
            <div
                class="inputs"
                id="inputs"
                onInput={onParentInput}
                onKeyDown={onParentKeyDown}
                onPaste={onParentPaste}
            >
                {Array.from({ length: props.length }).map((_, i) => {
                    return (
                        <input
                            maxlength={1}
                            type="text"
                            value={code()[i] ?? ""}
                            onInput={(e) => {
                                let newCode = code()
                                let newCodeArr = newCode.split("")
                                newCodeArr[i] = (
                                    e.target as HTMLInputElement
                                ).value
                                newCode = newCodeArr.join("")
                                setCode(newCode)
                                props.onChange(newCode)
                            }}
                        />
                    )
                })}
            </div>
        </div>
    )
}

export default ReceiveCodeInput
