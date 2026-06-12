package testapp

/** wasi:logging leg of the consolidation — the my:skiko-gfx log-message
 *  verb's post-Phase-B home (context tag mirrors the app id). */
internal fun logMessage(message: String) {
    wandr.platform.Logging.Import.log(
        wandr.platform.Logging.Level.INFO, "wandr-app", message,
    )
}
