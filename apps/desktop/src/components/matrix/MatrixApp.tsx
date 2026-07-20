// Top-level Matrix flow: restore a persisted session on mount, otherwise show login.
// Once logged in, start the background sync loop and render the chat.

import { useEffect, useState } from "preact/hooks"
import { api, type MatrixStatus } from "../../lib/api"
import { MatrixLogin } from "./MatrixLogin"
import { MatrixChat } from "./MatrixChat"

export function MatrixApp() {
	const [status, setStatus] = useState<MatrixStatus | null>(null)
	const [loading, setLoading] = useState(true)

	useEffect(() => {
		api.matrixRestoreSession()
			.then(setStatus)
			.catch(() => setStatus(null))
			.finally(() => setLoading(false))
	}, [])

	useEffect(() => {
		if (status?.logged_in) api.matrixStartSync().catch(() => {})
	}, [status?.logged_in])

	if (loading) return null

	if (!status?.logged_in) {
		return <MatrixLogin homeserver={status?.homeserver_url ?? ""} onLoggedIn={setStatus} />
	}

	return (
		<MatrixChat
			status={status}
			onLogout={async () => {
				await api.matrixLogout().catch(() => {})
				setStatus({ ...status, logged_in: false, user_id: null, device_id: null })
			}}
		/>
	)
}
