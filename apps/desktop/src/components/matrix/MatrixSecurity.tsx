// Security & recovery panel: shows cross-signing / device-verification status, this
// account's devices (with a one-click verify), and lets the user enable encrypted key
// backup (recovery) or restore it from a recovery key on a new device.

import { useEffect, useState } from "preact/hooks"
import type { UnlistenFn } from "@tauri-apps/api/event"
import {
	api,
	errMessage,
	type MatrixDevice,
	type MatrixEmoji,
	type MatrixVerificationStatus,
} from "../../lib/api"
import "./matrix.scss"

export function MatrixSecurity({ onClose }: { onClose: () => void }) {
	const [status, setStatus] = useState<MatrixVerificationStatus | null>(null)
	const [devices, setDevices] = useState<MatrixDevice[]>([])
	const [passphrase, setPassphrase] = useState("")
	const [recoveryKeyInput, setRecoveryKeyInput] = useState("")
	const [newKey, setNewKey] = useState("")
	const [busy, setBusy] = useState(false)
	const [error, setError] = useState("")
	const [note, setNote] = useState("")
	const [emojis, setEmojis] = useState<MatrixEmoji[] | null>(null)

	function refresh() {
		api.matrixVerificationStatus().then(setStatus).catch(() => {})
		api.matrixDevices().then(setDevices).catch(() => {})
	}
	useEffect(refresh, [])

	// Live verification events (emojis to compare, completion).
	useEffect(() => {
		const uns: UnlistenFn[] = []
		api.onVerificationEmojis((e) => setEmojis(e)).then((u) => uns.push(u))
		api.onVerificationDone(() => {
			setEmojis(null)
			setNote("Device verified ✅")
			refresh()
		}).then((u) => uns.push(u))
		api.onVerificationRequest(() => setNote("Incoming verification…")).then((u) => uns.push(u))
		return () => uns.forEach((u) => u())
	}, [])

	async function run(fn: () => Promise<void>) {
		setBusy(true)
		setError("")
		setNote("")
		try {
			await fn()
			refresh()
		} catch (err) {
			setError(errMessage(err))
		} finally {
			setBusy(false)
		}
	}

	return (
		<div class="mx-modal-backdrop" onClick={onClose}>
			<div class="mx-modal" onClick={(e) => e.stopPropagation()}>
				<h2>Security &amp; recovery</h2>

				{emojis && (
					<section class="mx-sec mx-verify">
						<h3>Compare emoji</h3>
						<p class="muted" style="font-size:12px">
							These should match on both devices. Confirm on each.
						</p>
						<div class="mx-emojis">
							{emojis.map((e, i) => (
								<div class="mx-emoji" key={i}>
									<span class="mx-emoji-sym">{e.symbol}</span>
									<span class="mx-emoji-desc">{e.description}</span>
								</div>
							))}
						</div>
						<div class="mx-new-actions">
							<button
								class="primary"
								disabled={busy}
								onClick={() => run(() => api.matrixConfirmVerification())}
							>
								They match
							</button>
							<button
								disabled={busy}
								onClick={() => {
									api.matrixCancelVerification().catch(() => {})
									setEmojis(null)
								}}
							>
								They don't
							</button>
						</div>
					</section>
				)}

				<section class="mx-sec">
					<h3>Status</h3>
					<div class="mx-sec-row">
						Cross-signing: {status?.cross_signing_ready ? "✅ ready" : "❌ not set up"}
					</div>
					<div class="mx-sec-row">
						This device: {status?.this_device_verified ? "✅ verified" : "❌ unverified"}
					</div>
					<div class="mx-sec-row">Recovery: {status?.recovery_state ?? "…"}</div>
				</section>

				<section class="mx-sec">
					<h3>Devices</h3>
					{devices.map((d) => (
						<div class="mx-sec-row mx-device" key={d.device_id}>
							<span>
								{d.display_name || d.device_id}
								{d.is_current && " (this device)"} — {d.verified ? "✅" : "unverified"}
							</span>
							<span class="mx-device-btns">
								{!d.verified && !d.is_current && (
									<>
										<button
											disabled={busy}
											title="Compare emoji with the other device"
											onClick={() => api.matrixRequestVerification(d.device_id).catch((e) => setError(errMessage(e)))}
										>
											Verify
										</button>
										<button
											disabled={busy}
											title="Mark trusted by cross-signing (no emoji check)"
											onClick={() => run(() => api.matrixVerifyDevice(d.device_id))}
										>
											Trust
										</button>
									</>
								)}
								{!d.is_current && (
									<button
										disabled={busy || !passphrase}
										title="Sign out & remove this device (needs your password below)"
										onClick={() => run(() => api.matrixDeleteDevices([d.device_id], passphrase))}
									>
										Remove
									</button>
								)}
							</span>
						</div>
					))}
				</section>

				<section class="mx-sec">
					<h3>Your password</h3>
					<p class="muted" style="font-size:12px">
						Required to set up cross-signing and to remove devices (both hit a
						password-protected server endpoint).
					</p>
					<input
						type="password"
						placeholder="Your login password"
						value={passphrase}
						onInput={(e) => setPassphrase(e.currentTarget.value)}
					/>
				</section>

				<section class="mx-sec">
					<h3>Set up cross-signing &amp; backup</h3>
					<p class="muted" style="font-size:12px">
						Creates your cross-signing keys + encrypted server backup (uses the password
						above), so other devices can be verified. You'll get a recovery key — save it.
					</p>
					<button
						class="primary"
						disabled={busy || !passphrase}
						onClick={() =>
							run(async () => {
								await api.matrixBootstrapCrossSigning(passphrase)
								const key = await api.matrixRecoveryEnable(passphrase)
								setNewKey(key)
							})
						}
					>
						Set up
					</button>
					{newKey && (
						<p class="mx-key">
							Recovery key (save this now):<br />
							<code>{newKey}</code>
						</p>
					)}
				</section>

				<section class="mx-sec">
					<h3>Restore on this device</h3>
					<input
						placeholder="Recovery key or passphrase"
						value={recoveryKeyInput}
						onInput={(e) => setRecoveryKeyInput(e.currentTarget.value)}
					/>
					<button
						disabled={busy || !recoveryKeyInput}
						onClick={() =>
							run(async () => {
								await api.matrixRecover(recoveryKeyInput)
								setRecoveryKeyInput("")
								setNote("Recovered.")
							})
						}
					>
						Recover
					</button>
				</section>

				{note && <p class="muted">{note}</p>}
				{error && <p class="error-text">{error}</p>}
				<div class="mx-modal-actions">
					<button onClick={onClose}>Close</button>
				</div>
			</div>
		</div>
	)
}
