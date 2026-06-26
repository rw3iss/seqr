// Create a group: name it and pick which friends to include. The backend generates
// the group key and distributes it (sealed) to each selected member.

import { useEffect, useState } from "preact/hooks"
import { api, errMessage, type Friend } from "../lib/api"
import "./CreateGroupModal.scss"

interface Props {
	onClose: () => void
	onCreated: () => void
}

export function CreateGroupModal({ onClose, onCreated }: Props) {
	const [friends, setFriends] = useState<Friend[]>([])
	const [name, setName] = useState("")
	const [selected, setSelected] = useState<Set<string>>(new Set())
	const [busy, setBusy] = useState(false)
	const [error, setError] = useState("")

	useEffect(() => {
		api.listFriends().then(setFriends).catch((e) => setError(errMessage(e)))
	}, [])

	function toggle(signing: string) {
		setSelected((prev) => {
			const next = new Set(prev)
			next.has(signing) ? next.delete(signing) : next.add(signing)
			return next
		})
	}

	async function create(e: Event) {
		e.preventDefault()
		setBusy(true)
		setError("")
		try {
			await api.createGroup(name.trim() || "Group", [...selected])
			onCreated()
			onClose()
		} catch (err) {
			setError(errMessage(err))
		} finally {
			setBusy(false)
		}
	}

	return (
		<div class="modal-backdrop" onClick={onClose}>
			<div class="modal" onClick={(e) => e.stopPropagation()}>
				<form class="modal-body" onSubmit={create}>
					<h3 class="group-title">New group</h3>
					<input
						placeholder="Group name"
						value={name}
						onInput={(e) => setName(e.currentTarget.value)}
					/>
					<p class="muted">Choose members</p>
					<div class="member-list">
						{friends.length === 0 && <p class="muted">Add friends first.</p>}
						{friends.map((f) => (
							<label key={f.signing_public} class="member-row">
								<input
									type="checkbox"
									checked={selected.has(f.signing_public)}
									onChange={() => toggle(f.signing_public)}
								/>
								<span>{f.display_name}</span>
							</label>
						))}
					</div>
					{error && <p class="error-text">{error}</p>}
					<button class="primary" type="submit" disabled={busy || selected.size === 0}>
						{busy ? "Creating…" : `Create group (${selected.size})`}
					</button>
				</form>
				<button class="modal-close" onClick={onClose}>Close</button>
			</div>
		</div>
	)
}
