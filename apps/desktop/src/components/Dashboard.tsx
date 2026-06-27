// The main two-pane dashboard: conversations (1:1 + groups) on the left, the active
// chat on the right. Direct and group messages share the same chat window; sends are
// routed by conversation kind.

import { useEffect, useRef, useState } from "preact/hooks"
import {
	api,
	errMessage,
	type AppConfig,
	type AttachmentInfo,
	type AttachmentProgress,
	type Conversation,
	type ProfileBlob,
	type Settings,
	type StoredMessage,
} from "../lib/api"
import { AddFriendModal } from "./AddFriendModal"
import { CreateGroupModal } from "./CreateGroupModal"
import { GroupMembersModal } from "./GroupMembersModal"
import { FriendRequests } from "./FriendRequests"
import { SettingsModal } from "./SettingsModal"
import { ensureNotificationPermission, notify } from "../lib/notify"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { open, save } from "@tauri-apps/plugin-dialog"
import type { Friend } from "../lib/api"
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
	const [showSettings, setShowSettings] = useState(false)
	const [config, setConfig] = useState<AppConfig | null>(null)
	const [settings, setSettings] = useState<Settings | null>(null)
	const [focused, setFocused] = useState(true)
	const [online, setOnline] = useState<Set<string>>(new Set())
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
		api.getSettings().then((s) => {
			setSettings(s)
			if (s.notifications_enabled) ensureNotificationPermission()
		}).catch(() => {})
		// A new group invite (or membership change) should surface in the list.
		const unlisten = api.onGroupUpdate(() => refresh())
		return () => {
			unlisten.then((fn) => fn())
		}
	}, [])

	// Poll the mailbox for which friends are online (recently active).
	useEffect(() => {
		let stop = false
		async function poll() {
			const peers = conversations
				.filter((c) => c.kind === "direct" && c.peer)
				.map((c) => c.peer as string)
			if (peers.length === 0) return
			try {
				const onlineIds = await api.presence(peers)
				if (!stop) setOnline(new Set(onlineIds))
			} catch {
				/* ignore transient errors */
			}
		}
		poll()
		const t = setInterval(poll, 8000)
		return () => {
			stop = true
			clearInterval(t)
		}
	}, [conversations])

	// Track window focus so we only notify when Seqr is in the background/minimized.
	useEffect(() => {
		const win = getCurrentWindow()
		win.isFocused().then(setFocused).catch(() => {})
		const unlisten = win.onFocusChanged(({ payload }) => setFocused(payload))
		return () => {
			unlisten.then((fn) => fn())
		}
	}, [])

	// Global notification listener: notify on inbound messages when the window isn't
	// focused (covers minimized/background), or when focused but viewing another chat.
	useEffect(() => {
		const unlisten = api.onMessage((m) => {
			if (!settings?.notifications_enabled) return
			const viewingThis = focused && selected?.id === m.conversation_id
			if (viewingThis) return
			const conv = conversations.find((c) => c.id === m.conversation_id)
			notify(conv?.title ?? "New message", m.body)
		})
		return () => {
			unlisten.then((fn) => fn())
		}
	}, [settings, selected, conversations, focused])

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
					<div class="head-buttons">
						<button class="icon-btn" title="Settings" onClick={() => setShowSettings(true)}>⚙</button>
						<button class="icon-btn" title="Lock" onClick={lock}>⏻</button>
					</div>
				</header>

				<div class="sidebar-actions">
					<button class="primary" onClick={() => setShowAdd(true)}>+ Friend</button>
					<button onClick={() => setShowGroup(true)}>+ Group</button>
				</div>

				<FriendRequests onChanged={refresh} />

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
							{c.kind === "direct" && (
								<span
									class={"presence" + (c.peer && online.has(c.peer) ? " online" : "")}
									title={c.peer && online.has(c.peer) ? "Online" : "Offline"}
								/>
							)}
						</button>
					))}
				</nav>

				{config && <footer class="sidebar-foot muted">mailbox: {config.mailbox_url}</footer>}
				{error && <p class="error-text">{error}</p>}
			</aside>

			<main class="chat">
				{selected ? (
					<ChatWindow
						key={selected.id}
						conversation={selected}
						settings={settings}
						onRemoved={() => {
							setSelected(null)
							refresh()
						}}
					/>
				) : (
					<div class="chat-empty muted">
						<h2>Seqr</h2>
						<p>Select a conversation to open your encrypted chat.</p>
					</div>
				)}
			</main>

			{showAdd && <AddFriendModal onClose={() => setShowAdd(false)} onFriendAdded={refresh} />}
			{showGroup && <CreateGroupModal onClose={() => setShowGroup(false)} onCreated={refresh} />}
			{showSettings && (
				<SettingsModal onClose={() => setShowSettings(false)} onSaved={setSettings} />
			)}
		</div>
	)
}

function ChatWindow({
	conversation,
	settings,
	onRemoved,
}: {
	conversation: Conversation
	settings: Settings | null
	onRemoved: () => void
}) {
	const [messages, setMessages] = useState<StoredMessage[]>([])
	const [progress, setProgress] = useState<Record<string, AttachmentProgress>>({})
	const [draft, setDraft] = useState("")
	const [staged, setStaged] = useState<string[]>([])
	const [sending, setSending] = useState(false)
	const [error, setError] = useState("")
	const [notice, setNotice] = useState("")
	const [members, setMembers] = useState<Friend[]>([])
	const [showMembers, setShowMembers] = useState(false)
	const historyRef = useRef<HTMLDivElement>(null)

	const isGroup = conversation.kind === "group"
	const enterSends = settings?.enter_sends ?? true

	function refreshMembers() {
		if (isGroup) api.groupMembers(conversation.id).then(setMembers).catch(() => {})
	}

	function nameOf(signing: string): string {
		return members.find((m) => m.signing_public === signing)?.display_name ?? `${signing.slice(0, 8)}…`
	}

	function flash(msg: string) {
		setNotice(msg)
		setTimeout(() => setNotice(""), 2500)
	}

	async function rotate() {
		try {
			conversation.kind === "group"
				? await api.rotateGroup(conversation.id)
				: await api.rotateDirect(conversation.peer!)
			flash("🔑 Key rotated")
		} catch (e) {
			setError(errMessage(e))
		}
	}

	async function revoke() {
		if (conversation.kind !== "direct") return
		try {
			await api.removeFriend(conversation.peer!)
			onRemoved()
		} catch (e) {
			setError(errMessage(e))
		}
	}

	async function verify() {
		try {
			const num = await api.safetyNumber(conversation.peer!)
			setNotice(`Safety number — compare with ${conversation.title}: ${num}`)
			setTimeout(() => setNotice(""), 12000)
		} catch (e) {
			setError(errMessage(e))
		}
	}

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
		refreshMembers()

		// Append inbound messages; clear any in-progress placeholder for a completed file.
		const unlisten = api.onMessage((m) => {
			if (m.conversation_id === conversation.id) {
				setMessages((prev) => [...prev, m])
				if (m.attachment) {
					setProgress((prev) => {
						const next = { ...prev }
						delete next[m.attachment!.id]
						return next
					})
				}
				scrollToEnd()
			}
		})
		// Track transfer progress for this conversation.
		const unProg = api.onAttachmentProgress((p) => {
			if (p.conversation_id !== conversation.id) return
			setProgress((prev) => ({ ...prev, [p.att_id]: p }))
			scrollToEnd()
		})
		return () => {
			unlisten.then((fn) => fn())
			unProg.then((fn) => fn())
		}
	}, [conversation.id])

	// Native Tauri file-drop gives real filesystem paths (HTML drop is intercepted).
	useEffect(() => {
		const un = getCurrentWebview().onDragDropEvent((event) => {
			const p = event.payload
			if (p.type === "drop") {
				setStaged((prev) => [...prev, ...p.paths])
			}
		})
		return () => {
			un.then((fn) => fn())
		}
	}, [conversation.id])

	async function pickFiles() {
		const sel = await open({ multiple: true })
		if (!sel) return
		setStaged((prev) => [...prev, ...(Array.isArray(sel) ? sel : [sel])])
	}

	function unstage(path: string) {
		setStaged((prev) => prev.filter((p) => p !== path))
	}

	async function send() {
		const body = draft.trim()
		if (!body && staged.length === 0) return
		setSending(true)
		setError("")
		try {
			// Attachments first, then the text line.
			for (const path of staged) {
				const msg = await api.sendAttachment(conversation.id, path)
				setMessages((prev) => [...prev, msg])
			}
			if (body) {
				const msg =
					conversation.kind === "group"
						? await api.sendGroupMessage(conversation.id, body)
						: await api.sendMessage(conversation.peer!, body)
				setMessages((prev) => [...prev, msg])
			}
			setDraft("")
			setStaged([])
			scrollToEnd()
		} catch (err) {
			setError(errMessage(err))
		} finally {
			setSending(false)
		}
	}

	// Enter/Shift+Enter behavior per the user's setting.
	function onKeyDown(e: KeyboardEvent) {
		if (e.key !== "Enter") return
		const sendCombo = enterSends ? !e.shiftKey : e.shiftKey
		if (sendCombo) {
			e.preventDefault()
			send()
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
				<div class="chat-actions">
					{isGroup && (
						<button title="View and manage members" onClick={() => setShowMembers(true)}>
							Members
						</button>
					)}
					{conversation.kind === "direct" && (
						<button title="Show the safety number to verify identity" onClick={verify}>
							Verify
						</button>
					)}
					<button title="Rotate the conversation key" onClick={rotate}>Rotate key</button>
					{conversation.kind === "direct" && (
						<button class="danger" title="Revoke and remove this friend" onClick={revoke}>
							Revoke
						</button>
					)}
				</div>
			</header>

			{notice && <p class="chat-notice">{notice}</p>}

			<div class="chat-history" ref={historyRef}>
				{messages.length === 0 && (
					<div class="chat-info muted">
						<p>🔒 End-to-end encrypted.</p>
						<p>Messages are sealed with your shared key.</p>
					</div>
				)}
				{messages.map((m, i) => {
					const p = m.attachment ? progress[m.attachment.id] : undefined
					const uploading = p && p.outgoing && p.received < p.total
					return (
						<div key={i} class={"bubble" + (m.outgoing ? " out" : " in")}>
							{isGroup && !m.outgoing && <span class="bubble-sender">{nameOf(m.sender)}</span>}
							{m.attachment && <AttachmentView att={m.attachment} />}
							{m.body && <span class="bubble-body">{m.body}</span>}
							{uploading && (
								<span class="bubble-progress muted">
									Uploading… {Math.round((p!.received / p!.total) * 100)}%
								</span>
							)}
							<span class="bubble-time">{formatTime(m.ts)}</span>
						</div>
					)
				})}

				{/* Incoming files still arriving (no message bubble yet). */}
				{Object.values(progress)
					.filter((p) => !p.outgoing && p.received < p.total)
					.map((p) => (
						<div key={p.att_id} class="bubble in">
							<span class="bubble-body">📎 Receiving {p.filename}…</span>
							<progress class="recv-progress" value={p.received} max={p.total} />
							<span class="bubble-progress muted">
								{Math.round((p.received / p.total) * 100)}% of {formatSize(p.size)}
							</span>
						</div>
					))}
			</div>

			{error && <p class="error-text chat-error">{error}</p>}

			{staged.length > 0 && (
				<div class="staged">
					{staged.map((p) => (
						<span key={p} class="staged-chip" title={p}>
							📎 {p.split(/[/\\]/).pop()}
							<button class="staged-x" onClick={() => unstage(p)}>×</button>
						</span>
					))}
				</div>
			)}

			<div class="composer">
				<button class="icon-btn attach-btn" title="Attach files" onClick={pickFiles}>📎</button>
				<textarea
					rows={1}
					placeholder={`Message ${conversation.title}…  (drag files here)`}
					value={draft}
					onInput={(e) => setDraft(e.currentTarget.value)}
					onKeyDown={onKeyDown}
				/>
				<button
					class="primary"
					onClick={send}
					disabled={sending || (!draft.trim() && staged.length === 0)}
				>
					Send
				</button>
			</div>

			{showMembers && (
				<GroupMembersModal
					groupId={conversation.id}
					groupName={conversation.title}
					onClose={() => setShowMembers(false)}
					onChanged={() => {
						refreshMembers()
						flash("Member removed · key rotated")
					}}
				/>
			)}
		</div>
	)
}

function formatTime(ts: number): string {
	const d = new Date(ts)
	return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
}

function formatSize(n: number): string {
	if (n < 1024) return `${n} B`
	if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`
	if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
	return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

async function downloadAttachment(att: AttachmentInfo) {
	try {
		const dest = await save({ defaultPath: att.filename })
		if (dest) await api.saveAttachment(att.id, dest)
	} catch {
		/* user cancelled */
	}
}

// Renders an attachment: image thumbnail (opens a preview modal), or a downloadable chip.
function AttachmentView({ att }: { att: AttachmentInfo }) {
	const [src, setSrc] = useState("")
	const [showModal, setShowModal] = useState(false)
	const isImage = att.mime.startsWith("image/")
	useEffect(() => {
		if (isImage) api.readAttachment(att.id).then(setSrc).catch(() => {})
	}, [att.id])

	if (isImage) {
		return (
			<>
				{src ? (
					<img
						class="att-image"
						src={src}
						alt={att.filename}
						title="Click to preview"
						onClick={() => setShowModal(true)}
					/>
				) : (
					<div class="att-loading muted">loading image…</div>
				)}
				{showModal && (
					<ImageModal att={att} src={src} onClose={() => setShowModal(false)} />
				)}
			</>
		)
	}
	return (
		<button class="att-file" title="Download" onClick={() => downloadAttachment(att)}>
			📄 <span class="att-name">{att.filename}</span>
			<span class="muted">({formatSize(att.size)})</span>
			<span class="att-dl">⬇</span>
		</button>
	)
}

// Full-size image preview with a download button.
function ImageModal({ att, src, onClose }: { att: AttachmentInfo; src: string; onClose: () => void }) {
	return (
		<div class="img-modal-backdrop" onClick={onClose}>
			<div class="img-modal" onClick={(e) => e.stopPropagation()}>
				<img class="img-modal-img" src={src} alt={att.filename} />
				<div class="img-modal-bar">
					<span class="muted">{att.filename} ({formatSize(att.size)})</span>
					<div class="img-modal-actions">
						<button class="primary" onClick={() => downloadAttachment(att)}>Download</button>
						<button onClick={onClose}>Close</button>
					</div>
				</div>
			</div>
		</div>
	)
}
