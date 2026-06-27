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
			// Apply screen-capture exclusion live (no-op on Linux).
			if (patch.screen_security !== undefined) await api.setScreenSecurity(patch.screen_security)
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
						<>
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
							<label class="setting-row">
								<div>
									<div class="setting-name">Shift+Enter inserts a new line</div>
									<div class="setting-desc muted">
										{settings.enter_sends
											? "Enter sends; Shift+Enter = new line."
											: "Enter = new line; Shift+Enter sends."}
									</div>
								</div>
								<input
									type="checkbox"
									checked={settings.enter_sends}
									disabled={saving}
									onChange={(e) => update({ enter_sends: e.currentTarget.checked })}
								/>
							</label>
							<label class="setting-row">
								<div>
									<div class="setting-name">Screen capture protection</div>
									<div class="setting-desc muted">
										Hide the window from screen recorders &amp; screenshots (macOS &amp;
										Windows; not supported on Linux).
									</div>
								</div>
								<input
									type="checkbox"
									checked={settings.screen_security}
									disabled={saving}
									onChange={(e) => update({ screen_security: e.currentTarget.checked })}
								/>
							</label>
						</>
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
