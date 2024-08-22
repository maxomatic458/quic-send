import { createSignal } from "solid-js";
import logo from "./assets/logo.svg";
import { invoke } from "@tauri-apps/api/tauri";
import { listen } from "@tauri-apps/api/event";
import { BiRegularUpload } from 'solid-icons/bi'
import "../App.css";
import { FileDropEvent } from "@tauri-apps/api/window";
import { Event } from "@tauri-apps/api/event";

function App() {
	const [sendField, setSendField] = createSignal<HTMLElement | null>(null);
	const [mainPage, setMainPage] = createSignal<HTMLElement | null>(null);

	listen('tauri://file-drop', async (event: Event<FileDropEvent>) => {
		let urlParam = "?files=";
		let paths = event.payload;
		let files = JSON.stringify(paths);
		urlParam += encodeURIComponent(files);
		window.location.href = "/send" + urlParam;
	})

	listen('tauri://file-drop-hover', async (_) => {
		const sendFieldRef = sendField();
		const mainPageRef = mainPage();
		if (sendFieldRef && mainPageRef) {
			sendFieldRef.classList.add('send-field-active');
			mainPageRef.classList.add('main-page-active');
		}
	})

	listen('tauri://file-drop-cancelled', async (_) => {
		const sendFieldRef = sendField();
		const mainPageRef = mainPage();
		if (sendFieldRef && mainPageRef) {
			sendFieldRef.classList.remove('send-field-active');
			mainPageRef.classList.remove('main-page-active');
		}
	})

	return (
		<div class="main-page" ref={(el) => setMainPage(el)}>
			<div class="send-field" ref={(el) => setSendField(el)}>
				<div class="upload-icon">
					<BiRegularUpload />
				</div>
				<div class="send-text">
					Drag and drop files here to send
				</div>
			</div>

			<div class="receive-button-container">
				<a href="/receive" class="receive-button">Receive files</a>
			</div>
		</div>
	);
}

export default App;
