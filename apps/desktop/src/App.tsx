// Top-level router: decides between the login screen and the dashboard based on
// whether an account exists and whether it is currently unlocked.

import { useEffect, useState } from "preact/hooks"
import { api, type ProfileBlob } from "./lib/api"
import { LoginView } from "./components/LoginView"
import { Dashboard } from "./components/Dashboard"

export default function App() {
	const [loading, setLoading] = useState(true)
	const [accountExists, setAccountExists] = useState(false)
	const [profile, setProfile] = useState<ProfileBlob | null>(null)

	useEffect(() => {
		api.appStatus()
			.then((s) => setAccountExists(s.account_exists))
			.finally(() => setLoading(false))
	}, [])

	if (loading) return null

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
