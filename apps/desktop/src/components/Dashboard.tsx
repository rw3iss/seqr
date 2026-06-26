// The main two-pane dashboard: friends list on the left, chat window on the right.
// Messaging itself (live transport + history) arrives in milestone 2; this shell
// renders the real roster and selection against the implemented backend.

import { useEffect, useRef, useState } from "preact/hooks"
import {
	api,
	errMessage,
	type AppConfig,
	type Friend,
	type ProfileBlob,
	type StoredMessage,
} from "../lib/api"
import { AddFriendModal } from "./AddFriendModal"
import "./Dashboard.scss"

interface Props {
	profile: ProfileBlob
	onLocked: () => void
}

export function Dashboard({ profile, onLocked }: Props) {
	const [friends, setFriends] = useState<Friend[]>([])
	const [selected, setSelected] = useState<Friend | null>(null)
	const [showAdd, setShowAdd] = useState(false)
	const [config, setConfig] = useState<AppConfig | null>(null)
	const [error, setError] = useState("")

	async function refresh() {
		try {
			setFriends(await api.listFriends())
		} catch (e) {
			setError(errMessage(e))
		}
	}

	useEffect(() => {
		refresh()
		api.appConfig().then(setConfig).catch(() => {})
	}, [])

	async function lock() {
		await api.lock()
		onLocked()
	}

	return (
		<div class="dashboard">
			<aside class="sidebar">
				<header class="sidebar-head">
					<div class="me">
						<span class="me-name">{profile.display_name}</span>
						<span class="me-status muted">● connected to mailbox</span>
					</div>
					<button class="icon-btn" title="Lock" onClick={lock}>⏻</button>
				</header>

				<button class="add-friend primary" onClick={() => setShowAdd(true)}>+ Add friend</button>

				<nav class="friends">
					{friends.length === 0 && (
						<p class="empty muted">No friends yet. Add one to start a private chat.</p>
					)}
					{friends.map((f) => (
						<button
							key={f.signing_public}
							class={"friend" + (selected?.signing_public === f.signing_public ? " selected" : "")}
							onClick={() => setSelected(f)}
						>
							<span class="friend-avatar">{f.display_name.charAt(0).toUpperCase()}</span>
							<span class="friend-name">{f.display_name}</span>
						</button>
					))}
				</nav>

				{config && <footer class="sidebar-foot muted">mailbox: {config.mailbox_url}</footer>}
				{error && <p class="error-text">{error}</p>}
			</aside>

			<main class="chat">
				{selected ? (
					<ChatWindow key={selected.signing_public} friend={selected} />
				) : (
					<div class="chat-empty muted">
						<h2>Seqr</h2>
						<p>Select a friend to open your encrypted conversation.</p>
					</div>
				)}
			</main>

			{showAdd && (
				<AddFriendModal onClose={() => setShowAdd(false)} onFriendAdded={refresh} />
			)}
		</div>
	)
}

function ChatWindow({ friend }: { friend: Friend }) {
	const [messages, setMessages] = useState<StoredMessage[]>([])
	const [draft, setDraft] = useState("")
	const [sending, setSending] = useState(false)
	const [error, setError] = useState("")
	const historyRef = useRef<HTMLDivElement>(null)

	function scrollToEnd() {
		requestAnimationFrame(() => {
			const el = historyRef.current
			if (el) el.scrollTop = el.scrollHeight
		})
	}

	useEffect(() => {
		api.getHistory(friend.signing_public)
			.then((m) => {
				setMessages(m)
				scrollToEnd()
			})
			.catch((e) => setError(errMessage(e)))

		// Append inbound messages from this friend as they arrive.
		const unlisten = api.onMessage((m) => {
			if (m.sender === friend.signing_public) {
				setMessages((prev) => [...prev, m])
				scrollToEnd()
			}
		})
		return () => {
			unlisten.then((fn) => fn())
		}
	}, [friend.signing_public])

	async function send(e: Event) {
		e.preventDefault()
		const body = draft.trim()
		if (!body) return
		setSending(true)
		setError("")
		try {
			const msg = await api.sendMessage(friend.signing_public, body)
			setMessages((prev) => [...prev, msg])
			setDraft("")
			scrollToEnd()
		} catch (err) {
			setError(errMessage(err))
		} finally {
			setSending(false)
		}
	}

	return (
		<div class="chat-window">
			<header class="chat-head">
				<span class="friend-avatar">{friend.display_name.charAt(0).toUpperCase()}</span>
				<div>
					<div class="chat-title">{friend.display_name}</div>
					<div class="chat-key muted">key: {friend.signing_public.slice(0, 16)}…</div>
				</div>
				<span class="badge secure">Secure · Static key</span>
			</header>

			<div class="chat-history" ref={historyRef}>
				{messages.length === 0 && (
					<div class="chat-info muted">
						<p>🔒 End-to-end encrypted with <strong>{friend.display_name}</strong>.</p>
						<p>Say hello — messages are sealed with your shared key.</p>
					</div>
				)}
				{messages.map((m, i) => (
					<div key={i} class={"bubble" + (m.outgoing ? " out" : " in")}>
						<span class="bubble-body">{m.body}</span>
						<span class="bubble-time">{formatTime(m.ts)}</span>
					</div>
				))}
			</div>

			{error && <p class="error-text chat-error">{error}</p>}

			<form class="composer" onSubmit={send}>
				<input
					placeholder={`Message ${friend.display_name}…`}
					value={draft}
					onInput={(e) => setDraft(e.currentTarget.value)}
				/>
				<button class="primary" type="submit" disabled={sending || !draft.trim()}>
					Send
				</button>
			</form>
		</div>
	)
}

function formatTime(ts: number): string {
	const d = new Date(ts)
	return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
}
