// Thin wrapper over Tauri's notification plugin: permission handling + sending.

import {
	isPermissionGranted,
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification"

/// Ensure we have OS permission to post notifications; prompts once if needed.
export async function ensureNotificationPermission(): Promise<boolean> {
	let granted = await isPermissionGranted()
	if (!granted) {
		granted = (await requestPermission()) === "granted"
	}
	return granted
}

/// Post a desktop notification (no-op if permission was denied).
export async function notify(title: string, body: string): Promise<void> {
	if (await isPermissionGranted()) {
		sendNotification({ title, body })
	}
}
