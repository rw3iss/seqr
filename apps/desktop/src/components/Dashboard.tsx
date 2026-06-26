// The main two-pane dashboard: friends list on the left, chat window on the right.
// Messaging itself (live transport + history) arrives in milestone 2; this shell
// renders the real roster and selection against the implemented backend.

import { useEffect, useState } from "preact/hooks"
import { api, errMessage, type AppConfig, type Friend, type ProfileBlob } from "../lib/api"
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
					<ChatPlaceholder friend={selected} />
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

function ChatPlaceholder({ friend }: { friend: Friend }) {
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

			<div class="chat-history">
				<div class="chat-info muted">
					<p>🔒 A secure channel with <strong>{friend.display_name}</strong> is ready to be established.</p>
					<p>Live messaging (direct iroh transport + offline mailbox delivery) lands in milestone 2.</p>
				</div>
			</div>

			<footer class="composer">
				<input placeholder="Messaging arrives in milestone 2…" disabled />
				<button class="primary" disabled>Send</button>
			</footer>
		</div>
	)
}
