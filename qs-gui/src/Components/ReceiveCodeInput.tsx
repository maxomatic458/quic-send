import { createSignal } from "solid-js"

interface ReceiveCodeInputProps {
    /// Callback for when the code changes
    onSubmit: (code: string) => void
}

function ReceiveCodeInput(props: ReceiveCodeInputProps) {
    const [code, setCode] = createSignal<string>("")

    return (
        <div class="receive-code-input-container">
            <div class="inputs" id="inputs">
                <input
                    spellcheck={false}
                    onPaste={(e) => {
                        e.preventDefault()
                        const pasteData = e.clipboardData?.getData("text") || ""
                        if (pasteData !== "") {
                            setCode(pasteData)
                            props.onSubmit(pasteData)
                        }
                    }}
                    onSubmit={(e) => {
                        e.preventDefault()
                        props.onSubmit(code())
                    }}
                    value={code()}
                    onInput={(e) => {
                        setCode((e.target as HTMLInputElement).value)
                    }}
                />
            </div>
        </div>
    )
}

export default ReceiveCodeInput
