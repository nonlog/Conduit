package com.conduit.sync

import android.content.Context
import android.os.Build

/** Localized platform Copy label list adapted from Sefirah's LanguageDetector. */
internal object ClipboardLocale {
    fun copyLabel(context: Context): String {
        val locale = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            context.resources.configuration.locales[0]
        } else {
            @Suppress("DEPRECATION")
            context.resources.configuration.locale
        }
        return copyLabels[locale.toLanguageTag()] ?: copyLabels[locale.language] ?: "Copy"
    }

    private val copyLabels = mapOf(
        "en" to "Copy", "ar" to "نسخ", "bg" to "Копиране", "bn" to "কপি করুন",
        "ca" to "Copia", "cs" to "Kopírovat", "da" to "Kopiér", "de" to "Kopieren",
        "el" to "Αντιγραφή", "es" to "Copiar", "et" to "Kopeerimine", "fa" to "کپی",
        "fi" to "Kopioi", "fr" to "Copier", "he" to "העתקה", "hi" to "कॉपी करें",
        "hr" to "Kopiraj", "hu" to "Másolás", "id" to "Salin", "it" to "Copia",
        "ja" to "コピー", "ko" to "복사", "lt" to "Kopijuoti", "lv" to "Kopēt",
        "ms" to "Salin", "nl" to "Kopiëren", "no" to "Kopiér", "pl" to "Kopiuj",
        "pt" to "Copiar", "ro" to "Copiați", "ru" to "Копировать", "sk" to "Kopírovať",
        "sl" to "Kopiraj", "sv" to "Kopiera", "ta" to "நகலெடு", "te" to "కాపీ చేయి",
        "th" to "คัดลอก", "tr" to "Kopyala", "uk" to "Скопіювати", "vi" to "Sao chép",
        "zh" to "复制", "zh-CN" to "复制", "zh-SG" to "复制",
        "zh-HK" to "複製", "zh-MO" to "複製", "zh-TW" to "複製",
    )
}
