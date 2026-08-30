package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ShareUrlTest {
    @Test fun accepts_bounded_http_and_https_pages() {
        assertEquals("https://rmpc.mierak.dev/", sharedWebUrl("  https://rmpc.mierak.dev/  "))
        assertEquals("http://example.com/a?b=1", sharedWebUrl("http://example.com/a?b=1"))
    }

    @Test fun ordinary_text_and_unsafe_schemes_stay_clipboard_text() {
        assertNull(sharedWebUrl("hello from Chrome"))
        assertNull(sharedWebUrl("file:///C:/secret.txt"))
        assertNull(sharedWebUrl("javascript:alert(1)"))
        assertNull(sharedWebUrl("https://example.com/\nnext"))
    }
    @Test fun chrome_page_share_prefers_url_over_auxiliary_preview_uri() {
        assertEquals(
            "https://example.com/article",
            sharedPageUrl(
                text = "https://example.com/article",
                title = "Example article",
                mimeType = "image/png",
                hasUris = true,
            ),
        )
        assertEquals(
            "https://example.com/article",
            sharedPageUrl(
                text = "https://example.com/article",
                title = null,
                mimeType = "text/plain",
                hasUris = true,
            ),
        )
    }

    @Test fun real_file_share_keeps_uri_precedence_without_page_signal() {
        assertNull(
            sharedPageUrl(
                text = "https://example.com/caption",
                title = null,
                mimeType = "image/jpeg",
                hasUris = true,
            ),
        )
    }
}
