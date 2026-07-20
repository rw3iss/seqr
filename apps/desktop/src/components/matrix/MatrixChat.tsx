// Matrix chat: room list on the left, the selected room's timeline + composer on the
// right. History loads on room switch; live messages arrive via the `matrix://message`
// event emitted by the background sync loop.

import { useEffect, useRef, useState } from "preact/hooks"
import type { UnlistenFn } from "@tauri-apps/api/event"
import { api, errMessage, type MatrixMessage, type MatrixRoom, type MatrixStatus } from "../../lib/api"
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
	const [error, setError] = useState("")

	// `active` captured for the (long-lived) event listener.
	const activeRef = useRef<string | null>(null)
	activeRef.current = active
	const bottomRef = useRef<HTMLDivElement>(null)

	function loadRooms() {
		api.matrixRooms()
			.then((r) => {
				setRooms(r)
				setActive((cur) => cur ?? (r[0]?.id ?? null))
			})
			.catch(() => {})
	}

	useEffect(() => {
		loadRooms()
		const id = setInterval(loadRooms, 10_000)
		return () => clearInterval(id)
	}, [])

	// Live inbound/echo messages for the open room, de-duplicated by event id.
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

	// Load history when switching rooms.
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

	const activeRoom = rooms.find((r) => r.id === active)

	return (
		<div class="mx">
			<aside class="mx-sidebar">
				<div class="mx-me">
					<div class="mx-me-id">{status.user_id}</div>
					<button class="mx-logout" onClick={onLogout}>Sign out</button>
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
					<div class="mx-empty">Select a room to start chatting.</div>
				) : (
					<>
						<div class="mx-messages">
							{messages.map((m) => (
								<div key={m.event_id ?? m.ts} class={`mx-msg${m.outgoing ? " out" : ""}`}>
									{!m.outgoing && <div class="mx-msg-sender">{m.sender}</div>}
									<div class="mx-msg-body">{m.body}</div>
								</div>
							))}
							<div ref={bottomRef} />
						</div>
						{error && <p class="error-text" style="padding:0 12px">{error}</p>}
						<form class="mx-composer" onSubmit={send}>
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
		</div>
	)
}
