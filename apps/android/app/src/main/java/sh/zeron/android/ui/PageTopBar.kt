package sh.zeron.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Height of [PageTopBar] below the status bar (New session's top app bar). */
val PageTopBarHeight = 64.dp

/** [PageTopBar]'s round buttons (New session's close button). */
val PageTopBarButton = 44.dp

/** How far below the header the transcript fades out (the composer's edge fade, mirrored). */
val TopEdgeFadeHeight = 44.dp

/**
 * A chat's top bar, sitting on the page with no container of its own, laid
 * out like New session's: a round leading button, the title with a status
 * line, round actions.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun PageTopBar(
    title: String,
    leading: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Row(
        modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .height(PageTopBarHeight)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading()
        Column(Modifier.weight(1f).padding(start = 16.dp, end = 8.dp)) {
            // New session's title face; a step smaller beside a status line
            // and up to four actions, so more of a chat's title fits.
            val style = MaterialTheme.typography.titleLargeEmphasized.let {
                if (subtitle != null) it.copy(fontSize = 18.sp, lineHeight = 24.sp) else it
            }
            Text(title, style = style, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (subtitle != null) {
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically, content = actions)
    }
}

/**
 * Content scrolled up under a [PageTopBar] fades out into the page instead of
 * meeting a hard edge. [scrolled] is how far the content is scrolled, in dp:
 * the fade comes in over the first [TopEdgeFadeHeight] so nothing at rest is
 * dimmed.
 */
@Composable
fun BoxScope.TopEdgeFade(scrolled: () -> Float) {
    val page = MaterialTheme.colorScheme.background
    Box(
        Modifier
            .align(Alignment.TopCenter)
            .fillMaxWidth()
            .height(TopEdgeFadeHeight)
            .graphicsLayer { alpha = (scrolled() / TopEdgeFadeHeight.value).coerceIn(0f, 1f) }
            .background(Brush.verticalGradient(0f to page.copy(alpha = 0.94f), 1f to page.copy(alpha = 0f))),
    )
}
