// Matrix chat: room list + create controls on the left; the selected room's timeline
// (text + media bubbles) and composer (with file attach) on the right. History loads on
// room switch; live messages arrive via the `matrix://message` sync event.

import { useEffect, useRef, useState } from "preact/hooks"
import type { UnlistenFn } from "@tauri-apps/api/event"
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog"
import { api, errMessage, type MatrixMessage, type MatrixRoom, type MatrixStatus } from "../../lib/api"
import { MatrixSecurity } from "./MatrixSecurity"
import "./matrix.scss"

interface Props {
	status: MatrixStatus
	onLogout: () => void
}

export function MatrixChat({ status, onLogout }: Props) {
	const [rooms, setRooms] = useState<MatrixRoom[]>([])
	const [active, setActive] = useState<string | null>(null)
	const [messages, setMessages] = useState<MatrixMessage[]>([])
	const [draft, setDraft] = useState("")
	const [newVal, setNewVal] = useState("")
	const [error, setError] = useState("")
	const [showSecurity, setShowSecurity] = useState(false)

	const activeRef = useRef<string | null>(null)
	activeRef.current = active
	const bottomRef = useRef<HTMLDivElement>(null)

	function loadRooms(select?: string) {
		api.matrixRooms()
			.then((r) => {
				setRooms(r)
				setActive((cur) => select ?? cur ?? r[0]?.id ?? null)
			})
			.catch(() => {})
	}

	useEffect(() => {
		loadRooms()
		const id = setInterval(() => loadRooms(), 10_000)
		return () => clearInterval(id)
	}, [])

	useEffect(() => {
		let un: UnlistenFn | undefined
		api.onMatrixMessage((m) => {
			if (m.room_id !== activeRef.current) return
			setMessages((prev) =>
				m.event_id && prev.some((p) => p.event_id === m.event_id) ? prev : [...prev, m],
			)
		}).then((f) => (un = f))
		return () => un?.()
	}, [])

	useEffect(() => {
		if (!active) {
			setMessages([])
			return
		}
		api.matrixRoomMessages(active).then(setMessages).catch(() => setMessages([]))
	}, [active])

	useEffect(() => {
		bottomRef.current?.scrollIntoView({ behavior: "smooth" })
	}, [messages])

	async function send(e: Event) {
		e.preventDefault()
		const body = draft.trim()
		if (!body || !active) return
		setDraft("")
		setError("")
		try {
			await api.matrixSendMessage(active, body)
		} catch (err) {
			setError(errMessage(err))
		}
	}

	async function startDm() {
		const id = newVal.trim()
		if (!id) return
		setNewVal("")
		try {
			const roomId = await api.matrixCreateDm(id)
			loadRooms(roomId)
		} catch (err) {
			setError(errMessage(err))
		}
	}

	async function createRoom() {
		const name = newVal.trim()
		if (!name) return
		setNewVal("")
		try {
			const roomId = await api.matrixCreateRoom(name, [])
			loadRooms(roomId)
		} catch (err) {
			setError(errMessage(err))
		}
	}

	async function attach() {
		if (!active) return
		const picked = await openDialog({ multiple: false })
		if (typeof picked !== "string") return
		try {
			await api.matrixSendFile(active, picked)
		} catch (err) {
			setError(errMessage(err))
		}
	}

	const activeRoom = rooms.find((r) => r.id === active)

	return (
		<div class={`mx${active ? " mx--room-open" : ""}`}>
			<aside class="mx-sidebar">
				<div class="mx-me">
					<div class="mx-me-id">{status.user_id}</div>
					<div class="mx-me-actions">
						<button class="mx-logout" onClick={() => setShowSecurity(true)}>Security</button>
						<button class="mx-logout" onClick={onLogout}>Sign out</button>
					</div>
				</div>

				<div class="mx-new">
					<input
						value={newVal}
						placeholder="@user:server or room name"
						onInput={(e) => setNewVal(e.currentTarget.value)}
					/>
					<div class="mx-new-actions">
						<button onClick={startDm} disabled={!newVal.trim()}>Start DM</button>
						<button onClick={createRoom} disabled={!newVal.trim()}>New room</button>
					</div>
				</div>

				<div class="mx-rooms">
					{rooms.length === 0 && <div class="mx-room-kind" style="padding:12px">No rooms yet.</div>}
					{rooms.map((r) => (
						<div
							key={r.id}
							class={`mx-room${r.id === active ? " active" : ""}`}
							onClick={() => setActive(r.id)}
						>
							<div class="mx-room-name">{r.name}</div>
							<div class="mx-room-kind">{r.is_dm ? "Direct" : "Room"}</div>
						</div>
					))}
				</div>
			</aside>

			<main class="mx-main">
				{!activeRoom ? (
					<div class="mx-empty">Select or start a conversation.</div>
				) : (
					<>
						<header class="mx-header">
							<button class="mx-back" onClick={() => setActive(null)} title="Back">‹</button>
							<span class="mx-room-name">{activeRoom.name}</span>
							<button
								class="mx-leave"
								onClick={async () => {
									if (!active) return
									await api.matrixLeave(active).catch(() => {})
									setActive(null)
									loadRooms()
								}}
							>
								Leave
							</button>
						</header>

						<div class="mx-messages">
							{messages.map((m) => (
								<div key={m.event_id ?? m.ts} class={`mx-msg${m.outgoing ? " out" : ""}`}>
									{!m.outgoing && <div class="mx-msg-sender">{m.sender}</div>}
									<MessageBody roomId={activeRoom.id} msg={m} onError={setError} />
									{m.ts > 0 && <div class="mx-msg-time">{formatTime(m.ts)}</div>}
								</div>
							))}
							<div ref={bottomRef} />
						</div>

						{error && <p class="error-text" style="padding:0 12px">{error}</p>}

						<form class="mx-composer" onSubmit={send}>
							<button type="button" class="mx-attach" onClick={attach} title="Attach a file">📎</button>
							<input
								value={draft}
								placeholder={`Message ${activeRoom.name}`}
								onInput={(e) => setDraft(e.currentTarget.value)}
							/>
							<button class="primary" type="submit" disabled={!draft.trim()}>Send</button>
						</form>
					</>
				)}
			</main>

			{showSecurity && <MatrixSecurity onClose={() => setShowSecurity(false)} />}
		</div>
	)
}

// Local-time HH:MM for a millisecond timestamp.
function formatTime(ts: number): string {
	return new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
}

// Renders a single message body: text inline, images lazily fetched + decrypted, other
// media as a download chip.
function MessageBody({
	roomId,
	msg,
	onError,
}: {
	roomId: string
	msg: MatrixMessage
	onError: (e: string) => void
}) {
	const [imgUrl, setImgUrl] = useState<string | null>(null)

	useEffect(() => {
		if (msg.msgtype === "m.image" && msg.event_id) {
			api.matrixReadMedia(roomId, msg.event_id).then(setImgUrl).catch(() => setImgUrl(null))
		}
	}, [msg.event_id, msg.msgtype])

	if (msg.msgtype === "m.text") return <div class="mx-msg-body">{msg.body}</div>

	if (msg.msgtype === "m.image") {
		return imgUrl ? (
			<img class="mx-msg-image" src={imgUrl} alt={msg.body} />
		) : (
			<div class="mx-msg-body">🖼️ {msg.body}</div>
		)
	}

	// Other media → download chip.
	async function download() {
		if (!msg.event_id) return
		const dest = await saveDialog({ defaultPath: msg.body })
		if (typeof dest !== "string") return
		try {
			await api.matrixSaveMedia(roomId, msg.event_id, dest)
		} catch (err) {
			onError(errMessage(err))
		}
	}

	return (
		<button class="mx-msg-file" onClick={download} title="Download">
			📎 {msg.body}
		</button>
	)
}
