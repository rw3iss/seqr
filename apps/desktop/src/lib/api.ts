// Typed wrappers around the Tauri IPC command surface. The UI talks only to these
// functions, never to `invoke` directly, so the command contract lives in one place.

import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"

export const MESSAGE_EVENT = "seqr://message"

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

export interface StoredMessage {
	conversation_id: string
	sender: string
	body: string
	ts: number
	outgoing: boolean
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
	getHistory: (friend: string): Promise<StoredMessage[]> => invoke("get_history", { friend }),
	sendMessage: (friend: string, body: string): Promise<StoredMessage> =>
		invoke("send_message", { friend, body }),
	onMessage: (cb: (m: StoredMessage) => void): Promise<UnlistenFn> =>
		listen<StoredMessage>(MESSAGE_EVENT, (e) => cb(e.payload)),
}

// Tauri rejects with the CoreError string; normalize for display.
export function errMessage(e: unknown): string {
	if (typeof e === "string") return e
	if (e && typeof e === "object" && "message" in e) return String((e as any).message)
	return "Something went wrong"
}
