package com.seqr.app

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // Screen-capture protection (parity with the desktop screen-security feature):
    // blocks screenshots, screen recording, and the app's preview thumbnail. Secure by
    // default for an E2E chat app; can be made toggleable via a plugin bridge later.
    window.setFlags(
      WindowManager.LayoutParams.FLAG_SECURE,
      WindowManager.LayoutParams.FLAG_SECURE,
    )
    super.onCreate(savedInstanceState)
  }
}
