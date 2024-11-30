import { Oval } from "solid-spinner"

function Loading(props: { text: string }) {
    return (
        <div class="loading">
            <div style={{ "margin-bottom": "1rem", "font-weight": "bold" }}>
                {props.text}
            </div>
            <div style={{ "text-align": "center" }} class="spinner">
                <Oval />
            </div>
        </div>
    )
}

export default Loading
