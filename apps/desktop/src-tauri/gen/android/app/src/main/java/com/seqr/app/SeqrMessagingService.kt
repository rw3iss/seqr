package com.seqr.app

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import java.io.File

/// Receives FCM lifecycle callbacks. The push payload is `event_id_only` (no content), so
/// on wake the app relies on the matrix-sdk sync loop to pull + decrypt the new event.
class SeqrMessagingService : FirebaseMessagingService() {
  override fun onNewToken(token: String) {
    // Persist the refreshed token for the Rust core to pick up and re-register the pusher.
    try {
      File(filesDir, "fcm_token").writeText(token)
    } catch (_: Exception) {}
  }

  override fun onMessageReceived(message: RemoteMessage) {
    // event_id_only wake. If the app process is alive its sync loop handles the new event.
    // A full background implementation (foreground sync service + system notification) is a
    // follow-up; noted in the cross-platform runbook.
  }
}
