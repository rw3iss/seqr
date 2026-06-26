// First-run account creation and subsequent unlock. The password is stretched into
// the vault key in the Rust core (Argon2id) — it never leaves the device.

import { useState } from "preact/hooks"
import { api, errMessage, type ProfileBlob } from "../lib/api"
import "./LoginView.scss"

interface Props {
	accountExists: boolean
	onUnlocked: (profile: ProfileBlob) => void
}

export function LoginView({ accountExists, onUnlocked }: Props) {
	const [displayName, setDisplayName] = useState("")
	const [password, setPassword] = useState("")
	const [busy, setBusy] = useState(false)
	const [error, setError] = useState("")

	async function submit(e: Event) {
		e.preventDefault()
		setBusy(true)
		setError("")
		try {
			const profile = accountExists
				? await api.unlock(password)
				: await api.createAccount(displayName.trim() || "Me", password)
			onUnlocked(profile)
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
					{accountExists ? "Welcome back. Unlock your vault." : "Create your local account."}
				</p>

				{!accountExists && (
					<label class="login-field">
						<span>Display name</span>
						<input
							value={displayName}
							placeholder="How friends will see you"
							onInput={(e) => setDisplayName(e.currentTarget.value)}
						/>
					</label>
				)}

				<label class="login-field">
					<span>Password</span>
					<input
						type="password"
						value={password}
						placeholder={accountExists ? "Your vault password" : "Choose a strong password"}
						onInput={(e) => setPassword(e.currentTarget.value)}
						autoFocus
					/>
				</label>

				{error && <p class="error-text login-error">{error}</p>}

				<button class="primary login-submit" type="submit" disabled={busy || !password}>
					{busy ? "Working…" : accountExists ? "Unlock" : "Create account"}
				</button>

				<p class="login-note muted">
					Your messages and keys are encrypted on this device. The password cannot be recovered.
				</p>
			</form>
		</div>
	)
}
