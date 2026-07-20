// Top-level router. First resolves the active backend from config: `matrix` (default)
// renders the Matrix client; `p2p` renders the legacy peer-to-peer flow (login → vault
// unlock → dashboard). Both backends are compiled into the Rust core.

import { useEffect, useState } from "preact/hooks"
import { api, type Backend, type ProfileBlob } from "./lib/api"
import { LoginView } from "./components/LoginView"
import { Dashboard } from "./components/Dashboard"
import { MatrixApp } from "./components/matrix/MatrixApp"

export default function App() {
	const [backend, setBackend] = useState<Backend | null>(null)

	// P2P-only state (unused when backend === "matrix").
	const [accountExists, setAccountExists] = useState(false)
	const [profile, setProfile] = useState<ProfileBlob | null>(null)

	useEffect(() => {
		api.appConfig()
			.then((c) => {
				setBackend(c.backend)
				if (c.backend === "p2p") {
					api.appStatus().then((s) => setAccountExists(s.account_exists))
				}
			})
			.catch(() => setBackend("matrix"))
	}, [])

	if (backend === null) return null

	if (backend === "matrix") return <MatrixApp />

	// Legacy P2P flow.
	if (!profile) {
		return (
			<LoginView
				accountExists={accountExists}
				onUnlocked={(p) => {
					setAccountExists(true)
					setProfile(p)
				}}
			/>
		)
	}
	return <Dashboard profile={profile} onLocked={() => setProfile(null)} />
}
