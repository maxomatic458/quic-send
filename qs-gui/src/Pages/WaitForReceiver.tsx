import { Oval } from "solid-spinner"

interface WaitForReceiverProps {
    code: string
}

function WaitForReceiver(props: WaitForReceiverProps) {
    return (
        <div style={{ height: "94vh" }}>
            <h3 class="text-center" style={{ "margin-top": "2rem" }}>
                Waiting for receiver to connect
            </h3>
            <div class="share-code-info">
                <h4 style={{ "margin-bottom": "1rem" }}>
                    Share the code or url below with the receiver
                </h4>

                <div
                    class="share-code text-center"
                    style={{ "margin-bottom": "1rem" }}
                >
                    {props.code}
                </div>

                <div class="text-center spinner">
                    <Oval />
                </div>
            </div>
        </div>
    )
}

export default WaitForReceiver
