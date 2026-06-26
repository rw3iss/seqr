// The main two-pane dashboard: conversations (1:1 + groups) on the left, the active
// chat on the right. Direct and group messages share the same chat window; sends are
// routed by conversation kind.

import { useEffect, useRef, useState } from "preact/hooks"
import {
	api,
	errMessage,
	type AppConfig,
	type Conversation,
	type ProfileBlob,
	type StoredMessage,
} from "../lib/api"
import { AddFriendModal } from "./AddFriendModal"
import { CreateGroupModal } from "./CreateGroupModal"
import "./Dashboard.scss"

interface Props {
	profile: ProfileBlob
	onLocked: () => void
}

export function Dashboard({ profile, onLocked }: Props) {
	const [conversations, setConversations] = useState<Conversation[]>([])
	const [selected, setSelected] = useState<Conversation | null>(null)
	const [showAdd, setShowAdd] = useState(false)
	const [showGroup, setShowGroup] = useState(false)
	const [config, setConfig] = useState<AppConfig | null>(null)
	const [error, setError] = useState("")

	async function refresh() {
		try {
			setConversations(await api.listConversations())
		} catch (e) {
			setError(errMessage(e))
		}
	}

	useEffect(() => {
		refresh()
		api.appConfig().then(setConfig).catch(() => {})
		// A new group invite (or membership change) should surface in the list.
		const unlisten = api.onGroupUpdate(() => refresh())
		return () => {
			unlisten.then((fn) => fn())
		}
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
						<span class="me-status muted">● connected</span>
					</div>
					<button class="icon-btn" title="Lock" onClick={lock}>⏻</button>
				</header>

				<div class="sidebar-actions">
					<button class="primary" onClick={() => setShowAdd(true)}>+ Friend</button>
					<button onClick={() => setShowGroup(true)}>+ Group</button>
				</div>

				<nav class="friends">
					{conversations.length === 0 && (
						<p class="empty muted">No conversations yet. Add a friend to begin.</p>
					)}
					{conversations.map((c) => (
						<button
							key={c.id}
							class={"friend" + (selected?.id === c.id ? " selected" : "")}
							onClick={() => setSelected(c)}
						>
							<span class={"friend-avatar" + (c.kind === "group" ? " group" : "")}>
								{c.kind === "group" ? "#" : c.title.charAt(0).toUpperCase()}
							</span>
							<span class="friend-name">{c.title}</span>
						</button>
					))}
				</nav>

				{config && <footer class="sidebar-foot muted">mailbox: {config.mailbox_url}</footer>}
				{error && <p class="error-text">{error}</p>}
			</aside>

			<main class="chat">
				{selected ? (
					<ChatWindow key={selected.id} conversation={selected} />
				) : (
					<div class="chat-empty muted">
						<h2>Seqr</h2>
						<p>Select a conversation to open your encrypted chat.</p>
					</div>
				)}
			</main>

			{showAdd && <AddFriendModal onClose={() => setShowAdd(false)} onFriendAdded={refresh} />}
			{showGroup && <CreateGroupModal onClose={() => setShowGroup(false)} onCreated={refresh} />}
		</div>
	)
}

function ChatWindow({ conversation }: { conversation: Conversation }) {
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
		api.getHistory(conversation.id)
			.then((m) => {
				setMessages(m)
				scrollToEnd()
			})
			.catch((e) => setError(errMessage(e)))

		// Append inbound messages belonging to this conversation.
		const unlisten = api.onMessage((m) => {
			if (m.conversation_id === conversation.id) {
				setMessages((prev) => [...prev, m])
				scrollToEnd()
			}
		})
		return () => {
			unlisten.then((fn) => fn())
		}
	}, [conversation.id])

	async function send(e: Event) {
		e.preventDefault()
		const body = draft.trim()
		if (!body) return
		setSending(true)
		setError("")
		try {
			const msg =
				conversation.kind === "group"
					? await api.sendGroupMessage(conversation.id, body)
					: await api.sendMessage(conversation.peer!, body)
			setMessages((prev) => [...prev, msg])
			setDraft("")
			scrollToEnd()
		} catch (err) {
			setError(errMessage(err))
		} finally {
			setSending(false)
		}
	}

	const subtitle =
		conversation.kind === "group"
			? `${conversation.members} members`
			: `key: ${conversation.peer?.slice(0, 16)}…`

	return (
		<div class="chat-window">
			<header class="chat-head">
				<span class={"friend-avatar" + (conversation.kind === "group" ? " group" : "")}>
					{conversation.kind === "group" ? "#" : conversation.title.charAt(0).toUpperCase()}
				</span>
				<div>
					<div class="chat-title">{conversation.title}</div>
					<div class="chat-key muted">{subtitle}</div>
				</div>
				<span class="badge secure">
					{conversation.kind === "group" ? "Secure · Group key" : "Secure · Static key"}
				</span>
			</header>

			<div class="chat-history" ref={historyRef}>
				{messages.length === 0 && (
					<div class="chat-info muted">
						<p>🔒 End-to-end encrypted.</p>
						<p>Messages are sealed with your shared key.</p>
					</div>
				)}
				{messages.map((m, i) => (
					<div key={i} class={"bubble" + (m.outgoing ? " out" : " in")}>
						{conversation.kind === "group" && !m.outgoing && (
							<span class="bubble-sender">{m.sender.slice(0, 8)}…</span>
						)}
						<span class="bubble-body">{m.body}</span>
						<span class="bubble-time">{formatTime(m.ts)}</span>
					</div>
				))}
			</div>

			{error && <p class="error-text chat-error">{error}</p>}

			<form class="composer" onSubmit={send}>
				<input
					placeholder={`Message ${conversation.title}…`}
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
