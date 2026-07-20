// Unlock screen shown at startup when an encrypted session exists on disk. The password
// derives (Argon2id) the key that decrypts the saved session in the Rust core.

import { useState } from "preact/hooks"
import { api, errMessage, type MatrixStatus } from "../../lib/api"
import "../LoginView.scss"

interface Props {
	userHint?: string | null
	onUnlocked: (s: MatrixStatus) => void
	onForget: () => void
}

export function MatrixUnlock({ userHint, onUnlocked, onForget }: Props) {
	const [password, setPassword] = useState("")
	const [busy, setBusy] = useState(false)
	const [error, setError] = useState("")

	async function submit(e: Event) {
		e.preventDefault()
		setBusy(true)
		setError("")
		try {
			onUnlocked(await api.matrixUnlock(password))
		} catch (err) {
			setError(errMessage(err))
		} finally {
			setBusy(false)
		}
	}

	return (
		<div class="login-view">
			<form class="login-card" onSubmit={submit}>
				<h1 class="login-brand">Seqr</h1>
				<p class="login-sub muted">
					{userHint ? `Welcome back, ${userHint}.` : "Welcome back."} Unlock your session.
				</p>

				<label class="login-field">
					<span>Password</span>
					<input
						type="password"
						value={password}
						placeholder="Your password"
						onInput={(e) => setPassword(e.currentTarget.value)}
						autoFocus
					/>
				</label>

				{error && <p class="error-text login-error">{error}</p>}

				<button class="primary login-submit" type="submit" disabled={busy || !password}>
					{busy ? "Unlocking…" : "Unlock"}
				</button>

				<button type="button" class="login-toggle" onClick={onForget}>
					Sign in as someone else
				</button>

				<p class="login-note muted">
					Your session is encrypted on this device; the password can't be recovered.
				</p>
			</form>
		</div>
	)
}
