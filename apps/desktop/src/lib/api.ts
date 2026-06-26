// Typed wrappers around the Tauri IPC command surface. The UI talks only to these
// functions, never to `invoke` directly, so the command contract lives in one place.

import { invoke } from "@tauri-apps/api/core"

export interface ProfileBlob {
	v: number
	display_name: string
	agreement_public: string
	signing_public: string
	node_addr: string
}

export interface Friend {
	display_name: string
	agreement_public: string
	signing_public: string
	node_addr: string
}

export interface AppStatus {
	account_exists: boolean
	unlocked: boolean
}

export interface AppConfig {
	mailbox_url: string
}

export const api = {
	appStatus: (): Promise<AppStatus> => invoke("app_status"),
	appConfig: (): Promise<AppConfig> => invoke("app_config"),
	createAccount: (displayName: string, password: string): Promise<ProfileBlob> =>
		invoke("create_account", { displayName, password }),
	unlock: (password: string): Promise<ProfileBlob> => invoke("unlock", { password }),
	lock: (): Promise<void> => invoke("lock"),
	myProfile: (): Promise<ProfileBlob> => invoke("my_profile"),
	exportProfile: (): Promise<string> => invoke("export_profile"),
	importFriend: (token: string): Promise<Friend> => invoke("import_friend", { token }),
	listFriends: (): Promise<Friend[]> => invoke("list_friends"),
}

// Tauri rejects with the CoreError string; normalize for display.
export function errMessage(e: unknown): string {
	if (typeof e === "string") return e
	if (e && typeof e === "object" && "message" in e) return String((e as any).message)
	return "Something went wrong"
}
