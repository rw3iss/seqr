// Typed wrappers around the Tauri IPC command surface. The UI talks only to these
// functions, never to `invoke` directly, so the command contract lives in one place.

import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"

export const MESSAGE_EVENT = "seqr://message"
export const GROUP_EVENT = "seqr://group"
export const REQUEST_EVENT = "seqr://request"
export const PROGRESS_EVENT = "seqr://attachment-progress"

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

export type Backend = "matrix" | "p2p"

export interface AppConfig {
	/** Active chat backend; the UI routes to the Matrix or P2P flow based on this. */
	backend: Backend
	/** Matrix homeserver base URL (used when backend === "matrix"). */
	homeserver_url: string
	mailbox_url: string
}

export interface MatrixStatus {
	homeserver_url: string
	logged_in: boolean
	user_id: string | null
	device_id: string | null
}

export interface MatrixRoom {
	id: string
	name: string
	is_dm: boolean
}

export interface MatrixReaction {
	key: string
	count: number
	mine: boolean
}

export interface MatrixMessage {
	room_id: string
	event_id: string | null
	sender: string
	body: string
	/** "m.text" | "m.image" | "m.file" | "m.video" | "m.audio" | … */
	msgtype: string
	ts: number
	outgoing: boolean
	reactions: MatrixReaction[]
}

export interface MatrixMember {
	user_id: string
	display_name: string | null
}

export interface MatrixDevice {
	device_id: string
	display_name: string | null
	verified: boolean
	is_current: boolean
}

export interface MatrixVerificationStatus {
	cross_signing_ready: boolean
	this_device_verified: boolean
	recovery_state: string
}

export interface MatrixEmoji {
	symbol: string
	description: string
}

export const VERIFICATION_EMOJIS_EVENT = "matrix://verification-emojis"
export const VERIFICATION_DONE_EVENT = "matrix://verification-done"
export const VERIFICATION_REQUEST_EVENT = "matrix://verification-request"

export const MATRIX_MESSAGE_EVENT = "matrix://message"
export const MATRIX_ROOM_UPDATED_EVENT = "matrix://room-updated"

export interface AttachmentInfo {
	id: string
	filename: string
	mime: string
	size: number
}

export interface AttachmentProgress {
	att_id: string
	conversation_id: string
	filename: string
	size: number
	received: number
	total: number
	outgoing: boolean
}

export interface StoredMessage {
	conversation_id: string
	sender: string
	body: string
	ts: number
	outgoing: boolean
	seq: number
	attachment: AttachmentInfo | null
}

export interface Conversation {
	id: string
	kind: "direct" | "group"
	title: string
	peer: string | null
	members: number
}

export interface Settings {
	notifications_enabled: boolean
	enter_sends: boolean
	screen_security: boolean
}

export const api = {
	appStatus: (): Promise<AppStatus> => invoke("app_status"),
	appConfig: (): Promise<AppConfig> => invoke("app_config"),

	// --- Matrix backend (active when appConfig().backend === "matrix") ---
	matrixStatus: (): Promise<MatrixStatus> => invoke("matrix_status"),
	matrixHasSession: (): Promise<boolean> => invoke("matrix_has_session"),
	matrixUnlock: (password: string): Promise<MatrixStatus> =>
		invoke("matrix_unlock", { password }),
	matrixLogin: (username: string, password: string): Promise<MatrixStatus> =>
		invoke("matrix_login", { username, password }),
	matrixRegister: (username: string, password: string, token: string): Promise<MatrixStatus> =>
		invoke("matrix_register", { username, password, token }),
	matrixLogout: (): Promise<void> => invoke("matrix_logout"),
	matrixStartSync: (): Promise<void> => invoke("matrix_start_sync"),
	matrixRooms: (): Promise<MatrixRoom[]> => invoke("matrix_rooms"),
	matrixRoomMessages: (roomId: string): Promise<MatrixMessage[]> =>
		invoke("matrix_room_messages", { roomId }),
	matrixSendMessage: (roomId: string, body: string): Promise<void> =>
		invoke("matrix_send_message", { roomId, body }),
	matrixReact: (roomId: string, eventId: string, key: string): Promise<void> =>
		invoke("matrix_react", { roomId, eventId, key }),
	matrixRedact: (roomId: string, eventId: string): Promise<void> =>
		invoke("matrix_redact", { roomId, eventId }),
	onMatrixRoomUpdated: (cb: (roomId: string) => void): Promise<UnlistenFn> =>
		listen<string>(MATRIX_ROOM_UPDATED_EVENT, (e) => cb(e.payload)),
	matrixCreateDm: (userId: string): Promise<string> => invoke("matrix_create_dm", { userId }),
	matrixCreateRoom: (name: string, invite: string[]): Promise<string> =>
		invoke("matrix_create_room", { name, invite }),
	matrixInvite: (roomId: string, userId: string): Promise<void> =>
		invoke("matrix_invite", { roomId, userId }),
	matrixJoin: (room: string): Promise<string> => invoke("matrix_join", { room }),
	matrixLeave: (roomId: string): Promise<void> => invoke("matrix_leave", { roomId }),
	matrixRoomMembers: (roomId: string): Promise<MatrixMember[]> =>
		invoke("matrix_room_members", { roomId }),
	matrixSendFile: (roomId: string, path: string): Promise<void> =>
		invoke("matrix_send_file", { roomId, path }),
	matrixReadMedia: (roomId: string, eventId: string): Promise<string> =>
		invoke("matrix_read_media", { roomId, eventId }),
	matrixSaveMedia: (roomId: string, eventId: string, dest: string): Promise<void> =>
		invoke("matrix_save_media", { roomId, eventId, dest }),
	matrixDevices: (): Promise<MatrixDevice[]> => invoke("matrix_devices"),
	matrixVerifyDevice: (deviceId: string): Promise<void> =>
		invoke("matrix_verify_device", { deviceId }),
	matrixRecoveryEnable: (passphrase: string): Promise<string> =>
		invoke("matrix_recovery_enable", { passphrase }),
	matrixRecover: (recoveryKey: string): Promise<void> =>
		invoke("matrix_recover", { recoveryKey }),
	matrixVerificationStatus: (): Promise<MatrixVerificationStatus> =>
		invoke("matrix_verification_status"),
	/** Register a push token with the homeserver (called from the mobile push SDK). */
	matrixRegisterPusher: (pushKey: string, appId: string): Promise<void> =>
		invoke("matrix_register_pusher", { pushKey, appId }),
	/** FCM token stashed by the Android layer; null on desktop. */
	matrixFcmToken: (): Promise<string | null> => invoke("matrix_fcm_token"),
	matrixRequestVerification: (deviceId: string): Promise<void> =>
		invoke("matrix_request_verification", { deviceId }),
	matrixConfirmVerification: (): Promise<void> => invoke("matrix_confirm_verification"),
	matrixCancelVerification: (): Promise<void> => invoke("matrix_cancel_verification"),
	onVerificationEmojis: (cb: (emojis: MatrixEmoji[]) => void): Promise<UnlistenFn> =>
		listen<MatrixEmoji[]>(VERIFICATION_EMOJIS_EVENT, (e) => cb(e.payload)),
	onVerificationDone: (cb: () => void): Promise<UnlistenFn> =>
		listen(VERIFICATION_DONE_EVENT, () => cb()),
	onVerificationRequest: (cb: (sender: string) => void): Promise<UnlistenFn> =>
		listen<string>(VERIFICATION_REQUEST_EVENT, (e) => cb(e.payload)),
	onMatrixMessage: (cb: (m: MatrixMessage) => void): Promise<UnlistenFn> =>
		listen<MatrixMessage>(MATRIX_MESSAGE_EVENT, (e) => cb(e.payload)),


	createAccount: (displayName: string, password: string): Promise<ProfileBlob> =>
		invoke("create_account", { displayName, password }),
	unlock: (password: string): Promise<ProfileBlob> => invoke("unlock", { password }),
	lock: (): Promise<void> => invoke("lock"),
	myProfile: (): Promise<ProfileBlob> => invoke("my_profile"),
	exportProfile: (): Promise<string> => invoke("export_profile"),
	importFriend: (token: string): Promise<Friend> => invoke("import_friend", { token }),
	listFriends: (): Promise<Friend[]> => invoke("list_friends"),
	getSettings: (): Promise<Settings> => invoke("get_settings"),
	setSettings: (settings: Settings): Promise<void> => invoke("set_settings", { settings }),
	setScreenSecurity: (enabled: boolean): Promise<void> =>
		invoke("set_screen_security", { enabled }),
	listRequests: (): Promise<Friend[]> => invoke("list_requests"),
	acceptRequest: (signing: string): Promise<void> => invoke("accept_request", { signing }),
	declineRequest: (signing: string): Promise<void> => invoke("decline_request", { signing }),
	listConversations: (): Promise<Conversation[]> => invoke("list_conversations"),
	presence: (ids: string[]): Promise<string[]> => invoke("presence", { ids }),
	getHistory: (conversationId: string): Promise<StoredMessage[]> =>
		invoke("get_history", { conversationId }),
	groupMembers: (groupId: string): Promise<Friend[]> => invoke("group_members", { groupId }),
	safetyNumber: (friend: string): Promise<string> => invoke("safety_number", { friend }),
	sendMessage: (friend: string, body: string): Promise<StoredMessage> =>
		invoke("send_message", { friend, body }),
	sendAttachment: (conversationId: string, path: string): Promise<StoredMessage> =>
		invoke("send_attachment", { conversationId, path }),
	readAttachment: (attId: string): Promise<string> => invoke("read_attachment", { attId }),
	saveAttachment: (attId: string, dest: string): Promise<void> =>
		invoke("save_attachment", { attId, dest }),
	stagePastedFile: (filename: string, data: string): Promise<string> =>
		invoke("stage_pasted_file", { filename, data }),
	createGroup: (name: string, members: string[]): Promise<Conversation> =>
		invoke("create_group", { name, members }),
	sendGroupMessage: (groupId: string, body: string): Promise<StoredMessage> =>
		invoke("send_group_message", { groupId, body }),
	rotateDirect: (friend: string): Promise<void> => invoke("rotate_direct", { friend }),
	removeFriend: (friend: string): Promise<void> => invoke("remove_friend", { friend }),
	rotateGroup: (groupId: string): Promise<void> => invoke("rotate_group", { groupId }),
	removeMember: (groupId: string, member: string): Promise<void> =>
		invoke("remove_member", { groupId, member }),
	onMessage: (cb: (m: StoredMessage) => void): Promise<UnlistenFn> =>
		listen<StoredMessage>(MESSAGE_EVENT, (e) => cb(e.payload)),
	onGroupUpdate: (cb: (id: string) => void): Promise<UnlistenFn> =>
		listen<string>(GROUP_EVENT, (e) => cb(e.payload)),
	onRequest: (cb: (signing: string) => void): Promise<UnlistenFn> =>
		listen<string>(REQUEST_EVENT, (e) => cb(e.payload)),
	onAttachmentProgress: (cb: (p: AttachmentProgress) => void): Promise<UnlistenFn> =>
		listen<AttachmentProgress>(PROGRESS_EVENT, (e) => cb(e.payload)),
}

// Tauri rejects with the CoreError string; normalize for display.
export function errMessage(e: unknown): string {
	if (typeof e === "string") return e
	if (e && typeof e === "object" && "message" in e) return String((e as any).message)
	return "Something went wrong"
}
