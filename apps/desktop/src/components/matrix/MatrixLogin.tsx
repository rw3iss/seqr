// Matrix sign-in / registration. Username is the localpart (or full @user:server). Login
// sends the password to the homeserver over TLS; registration also needs a one-time token
// (from the server's conduwuit.toml). Nothing is stored in plaintext beyond the session.

import { useState } from "preact/hooks"
import { api, errMessage, type MatrixStatus } from "../../lib/api"
import "../LoginView.scss"

interface Props {
	homeserver: string
	onLoggedIn: (s: MatrixStatus) => void
}

export function MatrixLogin({ homeserver, onLoggedIn }: Props) {
	const [mode, setMode] = useState<"login" | "register">("login")
	const [username, setUsername] = useState("")
	const [password, setPassword] = useState("")
	const [token, setToken] = useState("")
	const [busy, setBusy] = useState(false)
	const [error, setError] = useState("")

	const isRegister = mode === "register"

	async function submit(e: Event) {
		e.preventDefault()
		setBusy(true)
		setError("")
		try {
			const status = isRegister
				? await api.matrixRegister(username.trim(), password, token.trim())
				: await api.matrixLogin(username.trim(), password)
			onLoggedIn(status)
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
					{isRegister ? "Create a Matrix account." : "Sign in to your Matrix account."}
				</p>

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
						placeholder={isRegister ? "Choose a strong password" : "Your password"}
						onInput={(e) => setPassword(e.currentTarget.value)}
					/>
				</label>

				{isRegister && (
					<label class="login-field">
						<span>Registration token</span>
						<input
							value={token}
							placeholder="Invite token from the server admin"
							onInput={(e) => setToken(e.currentTarget.value)}
						/>
					</label>
				)}

				{error && <p class="error-text login-error">{error}</p>}

				<button
					class="primary login-submit"
					type="submit"
					disabled={busy || !username || !password || (isRegister && !token)}
				>
					{busy ? "Working…" : isRegister ? "Create account" : "Sign in"}
				</button>

				<button
					type="button"
					class="login-toggle"
					onClick={() => {
						setMode(isRegister ? "login" : "register")
						setError("")
					}}
				>
					{isRegister ? "Have an account? Sign in" : "Need an account? Register"}
				</button>

				<p class="login-note muted">Homeserver: {homeserver || "(not configured)"}</p>
			</form>
		</div>
	)
}
