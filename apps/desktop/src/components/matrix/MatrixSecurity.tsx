// Security & recovery panel: shows cross-signing / device-verification status, this
// account's devices (with a one-click verify), and lets the user enable encrypted key
// backup (recovery) or restore it from a recovery key on a new device.

import { useEffect, useState } from "preact/hooks"
import {
	api,
	errMessage,
	type MatrixDevice,
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

	function refresh() {
		api.matrixVerificationStatus().then(setStatus).catch(() => {})
		api.matrixDevices().then(setDevices).catch(() => {})
	}
	useEffect(refresh, [])

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
							{!d.verified && (
								<button
									disabled={busy}
									onClick={() => run(() => api.matrixVerifyDevice(d.device_id))}
								>
									Verify
								</button>
							)}
						</div>
					))}
				</section>

				<section class="mx-sec">
					<h3>Enable key backup</h3>
					<p class="muted" style="font-size:12px">
						Bootstraps cross-signing + encrypted server backup, protected by a passphrase.
						You'll get a recovery key — save it.
					</p>
					<input
						type="password"
						placeholder="Recovery passphrase"
						value={passphrase}
						onInput={(e) => setPassphrase(e.currentTarget.value)}
					/>
					<button
						class="primary"
						disabled={busy || !passphrase}
						onClick={() =>
							run(async () => {
								const key = await api.matrixRecoveryEnable(passphrase)
								setNewKey(key)
								setPassphrase("")
							})
						}
					>
						Enable
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
