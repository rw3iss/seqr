// Matrix password login. Username is the localpart (or full @user:server); the password
// is sent to the homeserver over TLS — nothing is stored in plaintext beyond the session.

import { useState } from "preact/hooks"
import { api, errMessage, type MatrixStatus } from "../../lib/api"
import "../LoginView.scss"

interface Props {
	homeserver: string
	onLoggedIn: (s: MatrixStatus) => void
}

export function MatrixLogin({ homeserver, onLoggedIn }: Props) {
	const [username, setUsername] = useState("")
	const [password, setPassword] = useState("")
	const [busy, setBusy] = useState(false)
	const [error, setError] = useState("")

	async function submit(e: Event) {
		e.preventDefault()
		setBusy(true)
		setError("")
		try {
			onLoggedIn(await api.matrixLogin(username.trim(), password))
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
				<p class="login-sub muted">Sign in to your Matrix account.</p>

				<label class="login-field">
					<span>Username</span>
					<input
						value={username}
						placeholder="you"
						onInput={(e) => setUsername(e.currentTarget.value)}
						autoFocus
					/>
				</label>

				<label class="login-field">
					<span>Password</span>
					<input
						type="password"
						value={password}
						placeholder="Your password"
						onInput={(e) => setPassword(e.currentTarget.value)}
					/>
				</label>

				{error && <p class="error-text login-error">{error}</p>}

				<button class="primary login-submit" type="submit" disabled={busy || !username || !password}>
					{busy ? "Signing in…" : "Sign in"}
				</button>

				<p class="login-note muted">
					Homeserver: {homeserver || "(not configured)"}
				</p>
			</form>
		</div>
	)
}
