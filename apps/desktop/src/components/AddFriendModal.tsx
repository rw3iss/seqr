// Add-a-friend modal with two tabs: export your own profile token to share, and
// import a friend's token to add them. Tokens are public-only (`seqr:<hex>`).

import { useEffect, useState } from "preact/hooks"
import { api, errMessage } from "../lib/api"
import "./AddFriendModal.scss"

interface Props {
	onClose: () => void
	onFriendAdded: () => void
}

export function AddFriendModal({ onClose, onFriendAdded }: Props) {
	const [tab, setTab] = useState<"export" | "import">("export")
	const [myToken, setMyToken] = useState("")
	const [importToken, setImportToken] = useState("")
	const [copied, setCopied] = useState(false)
	const [error, setError] = useState("")
	const [busy, setBusy] = useState(false)

	useEffect(() => {
		api.exportProfile().then(setMyToken).catch((e) => setError(errMessage(e)))
	}, [])

	async function copy() {
		await navigator.clipboard.writeText(myToken)
		setCopied(true)
		setTimeout(() => setCopied(false), 1500)
	}

	async function doImport(e: Event) {
		e.preventDefault()
		setBusy(true)
		setError("")
		try {
			await api.importFriend(importToken.trim())
			onFriendAdded()
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
				<div class="modal-tabs">
					<button class={tab === "export" ? "active" : ""} onClick={() => setTab("export")}>
						My profile
					</button>
					<button class={tab === "import" ? "active" : ""} onClick={() => setTab("import")}>
						Add a friend
					</button>
				</div>

				{tab === "export" ? (
					<div class="modal-body">
						<p class="muted">Share this token with a friend so they can add you.</p>
						<textarea class="token-box" readOnly value={myToken} rows={5} />
						<button class="primary" onClick={copy}>{copied ? "Copied!" : "Copy token"}</button>
					</div>
				) : (
					<form class="modal-body" onSubmit={doImport}>
						<p class="muted">Paste your friend's profile token.</p>
						<textarea
							class="token-box"
							placeholder="seqr:…"
							rows={5}
							value={importToken}
							onInput={(e) => setImportToken(e.currentTarget.value)}
						/>
						<button class="primary" type="submit" disabled={busy || !importToken.trim()}>
							{busy ? "Adding…" : "Add friend"}
						</button>
					</form>
				)}

				{error && <p class="error-text modal-error">{error}</p>}
				<button class="modal-close" onClick={onClose}>Close</button>
			</div>
		</div>
	)
}
