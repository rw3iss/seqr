// Top-level Matrix flow. At startup: if an encrypted session exists on disk, show the
// unlock screen (password decrypts it); otherwise show login/registration. Once a client
// is live, start the background sync loop and render the chat.

import { useEffect, useState } from "preact/hooks"
import { api, type MatrixStatus } from "../../lib/api"
import { MatrixLogin } from "./MatrixLogin"
import { MatrixUnlock } from "./MatrixUnlock"
import { MatrixChat } from "./MatrixChat"

export function MatrixApp() {
	const [status, setStatus] = useState<MatrixStatus | null>(null)
	const [homeserver, setHomeserver] = useState("")
	const [hasSession, setHasSession] = useState(false)
	const [loading, setLoading] = useState(true)

	useEffect(() => {
		Promise.all([api.matrixStatus(), api.matrixHasSession()])
			.then(([s, has]) => {
				setHomeserver(s.homeserver_url)
				setHasSession(has)
				if (s.logged_in) setStatus(s)
			})
			.catch(() => {})
			.finally(() => setLoading(false))
	}, [])

	useEffect(() => {
		if (!status?.logged_in) return
		api.matrixStartSync().catch(() => {})
		// On mobile, register the FCM pusher so the homeserver can wake us for new messages.
		// On desktop there's no token → no-op.
		api.matrixFcmToken()
			.then((token) => {
				if (token) return api.matrixRegisterPusher(token, "com.seqr.app.android")
			})
			.catch(() => {})
	}, [status?.logged_in])

	if (loading) return null

	if (status?.logged_in) {
		return (
			<MatrixChat
				status={status}
				onLogout={async () => {
					await api.matrixLogout().catch(() => {})
					setStatus(null)
					setHasSession(false)
				}}
			/>
		)
	}

	if (hasSession) {
		return (
			<MatrixUnlock
				onUnlocked={setStatus}
				onForget={async () => {
					await api.matrixLogout().catch(() => {})
					setHasSession(false)
				}}
			/>
		)
	}

	return <MatrixLogin homeserver={homeserver} onLoggedIn={setStatus} />
}
