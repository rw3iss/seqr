// View a group's members and remove any of them. Removing a member rotates the group
// key (excluding them), cutting them off from future messages.

import { useEffect, useState } from "preact/hooks"
import { api, errMessage, type Friend } from "../lib/api"
import "./GroupMembersModal.scss"

interface Props {
	groupId: string
	groupName: string
	onClose: () => void
	onChanged: () => void
}

export function GroupMembersModal({ groupId, groupName, onClose, onChanged }: Props) {
	const [members, setMembers] = useState<Friend[]>([])
	const [busy, setBusy] = useState("")
	const [error, setError] = useState("")

	async function refresh() {
		try {
			setMembers(await api.groupMembers(groupId))
		} catch (e) {
			setError(errMessage(e))
		}
	}

	useEffect(() => {
		refresh()
	}, [groupId])

	async function remove(signing: string) {
		setBusy(signing)
		setError("")
		try {
			await api.removeMember(groupId, signing)
			await refresh()
			onChanged()
		} catch (e) {
			setError(errMessage(e))
		} finally {
			setBusy("")
		}
	}

	return (
		<div class="modal-backdrop" onClick={onClose}>
			<div class="modal" onClick={(e) => e.stopPropagation()}>
				<div class="modal-body">
					<h3 class="group-title">{groupName} — members</h3>
					<div class="member-list">
						{members.length === 0 && <p class="muted">No other members.</p>}
						{members.map((m) => (
							<div key={m.signing_public} class="gm-member-row">
								<span>{m.display_name}</span>
								<button
									class="danger"
									disabled={busy === m.signing_public}
									onClick={() => remove(m.signing_public)}
								>
									{busy === m.signing_public ? "Removing…" : "Remove"}
								</button>
							</div>
						))}
					</div>
					{error && <p class="error-text">{error}</p>}
				</div>
				<button class="modal-close" onClick={onClose}>Close</button>
			</div>
		</div>
	)
}
