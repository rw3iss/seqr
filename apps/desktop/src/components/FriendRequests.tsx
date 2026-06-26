// Pending incoming friend requests. Shows the requester's name and safety number so
// the user can verify identity out-of-band before accepting.

import { useEffect, useState } from "preact/hooks"
import { api, errMessage, type Friend } from "../lib/api"
import "./FriendRequests.scss"

export function FriendRequests({ onChanged }: { onChanged: () => void }) {
	const [requests, setRequests] = useState<Friend[]>([])
	const [safety, setSafety] = useState<Record<string, string>>({})
	const [busy, setBusy] = useState("")
	const [error, setError] = useState("")

	async function refresh() {
		try {
			const reqs = await api.listRequests()
			setRequests(reqs)
			// Fetch safety numbers for verification.
			const pairs = await Promise.all(
				reqs.map(async (r) => [r.signing_public, await api.safetyNumber(r.signing_public)] as const),
			)
			setSafety(Object.fromEntries(pairs))
		} catch (e) {
			setError(errMessage(e))
		}
	}

	useEffect(() => {
		refresh()
		const unlisten = api.onRequest(() => refresh())
		return () => {
			unlisten.then((fn) => fn())
		}
	}, [])

	async function act(signing: string, accept: boolean) {
		setBusy(signing)
		setError("")
		try {
			accept ? await api.acceptRequest(signing) : await api.declineRequest(signing)
			await refresh()
			if (accept) onChanged()
		} catch (e) {
			setError(errMessage(e))
		} finally {
			setBusy("")
		}
	}

	if (requests.length === 0) return null

	return (
		<div class="requests">
			<div class="requests-head">Friend requests</div>
			{requests.map((r) => (
				<div key={r.signing_public} class="request-card">
					<div class="request-name">{r.display_name}</div>
					<div class="request-safety" title="Compare this with your friend to verify identity">
						{safety[r.signing_public] ?? "…"}
					</div>
					<div class="request-actions">
						<button
							class="primary"
							disabled={busy === r.signing_public}
							onClick={() => act(r.signing_public, true)}
						>
							Accept
						</button>
						<button disabled={busy === r.signing_public} onClick={() => act(r.signing_public, false)}>
							Decline
						</button>
					</div>
				</div>
			))}
			{error && <p class="error-text">{error}</p>}
		</div>
	)
}
