package com.tinyandr.smsbridge.util

import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/**
 * The theme draws edge to edge (transparent status bar, custom gradient
 * background). Without this the content runs under the status bar / nav bar.
 */
fun applySystemBarPadding(root: View) {
    val baseLeft = root.paddingLeft
    val baseTop = root.paddingTop
    val baseRight = root.paddingRight
    val baseBottom = root.paddingBottom
    if (root is android.widget.ScrollView) {
        root.clipToPadding = false
    }
    ViewCompat.setOnApplyWindowInsetsListener(root) { v, insets ->
        val bars = insets.getInsets(
            WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
        )
        v.setPadding(
            baseLeft + bars.left,
            baseTop + bars.top,
            baseRight + bars.right,
            baseBottom + bars.bottom
        )
        insets
    }
}
