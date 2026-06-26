// Settings panel. Currently: desktop-notification toggle. Settings persist in the
// encrypted vault (per account).

import { useEffect, useState } from "preact/hooks"
import { api, errMessage, type Settings } from "../lib/api"
import { ensureNotificationPermission } from "../lib/notify"
import "./SettingsModal.scss"

export function SettingsModal({
	onClose,
	onSaved,
}: {
	onClose: () => void
	onSaved: (s: Settings) => void
}) {
	const [settings, setSettings] = useState<Settings | null>(null)
	const [error, setError] = useState("")
	const [saving, setSaving] = useState(false)

	useEffect(() => {
		api.getSettings().then(setSettings).catch((e) => setError(errMessage(e)))
	}, [])

	async function update(patch: Partial<Settings>) {
		if (!settings) return
		const next = { ...settings, ...patch }
		setSettings(next)
		setSaving(true)
		setError("")
		try {
			// Asking the OS for permission the moment notifications are enabled.
			if (patch.notifications_enabled) await ensureNotificationPermission()
			await api.setSettings(next)
			onSaved(next)
		} catch (e) {
			setError(errMessage(e))
		} finally {
			setSaving(false)
		}
	}

	return (
		<div class="modal-backdrop" onClick={onClose}>
			<div class="modal" onClick={(e) => e.stopPropagation()}>
				<div class="modal-body">
					<h3 class="settings-title">Settings</h3>
					{settings ? (
						<label class="setting-row">
							<div>
								<div class="setting-name">Desktop notifications</div>
								<div class="setting-desc muted">Show a notification when a message arrives.</div>
							</div>
							<input
								type="checkbox"
								checked={settings.notifications_enabled}
								disabled={saving}
								onChange={(e) =>
									update({ notifications_enabled: e.currentTarget.checked })
								}
							/>
						</label>
					) : (
						<p class="muted">Loading…</p>
					)}
					{error && <p class="error-text">{error}</p>}
				</div>
				<button class="modal-close" onClick={onClose}>Close</button>
			</div>
		</div>
	)
}
