package sh.zeron.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilterChip
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.userMessage
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.CoreException

/**
 * Pi's custom model providers (`models.json` on the device), spoken over the
 * untyped `host_call` passthrough: `ListPiProviders`, `SavePiProvider`,
 * `RemovePiProvider`, `DiscoverPiProviderModels`. Keys stay on the device —
 * rows only carry their last four characters.
 */
object PiProviders {
    const val LIST = "ListPiProviders"
    const val SAVE = "SavePiProvider"
    const val REMOVE = "RemovePiProvider"
    const val DISCOVER = "DiscoverPiProviderModels"

    /** The wire protocols offered for new providers, with their labels. */
    val APIS = listOf("openai-completions" to "Chat Completions", "openai-responses" to "Responses")

    data class Provider(
        val name: String,
        val baseUrl: String,
        val api: String,
        /** `literal` | `env` | `command` | `none`. */
        val keySource: String,
        val keyTail: String?,
        val models: List<Model>,
    ) {
        val host: String get() = runCatching { java.net.URI(baseUrl).host }.getOrNull() ?: baseUrl
        val keyLabel: String get() = when (keySource) {
            "literal" -> keyTail?.let { "key ••••$it" } ?: "key set"
            "env" -> "key from environment"
            "command" -> "key from command"
            else -> "no key"
        }
    }

    data class Model(val id: String, val name: String?)

    fun providers(reply: Any): List<Provider> {
        val arr = reply as? JSONArray ?: return emptyList()
        return (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            val models = o.optJSONArray("models") ?: JSONArray()
            Provider(
                name = o.optString("name"),
                baseUrl = o.optString("baseUrl"),
                api = o.optString("api"),
                keySource = o.optString("keySource", "none"),
                keyTail = if (o.isNull("keyTail")) null else o.optString("keyTail").ifEmpty { null },
                models = (0 until models.length()).mapNotNull { j ->
                    val m = models.optJSONObject(j) ?: return@mapNotNull null
                    Model(m.optString("id"), if (m.isNull("name")) null else m.optString("name").ifEmpty { null })
                },
            )
        }
    }

    fun discovered(reply: Any): List<String> {
        val arr = (reply as? JSONObject)?.optJSONArray("models") ?: return emptyList()
        return (0 until arr.length()).map { arr.optString(it) }.filter { it.isNotEmpty() }
    }
}

/**
 * Settings → device → Accounts, under Pi: the device's custom providers and
 * a way to add one. Hidden when the device's engine predates them.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun PiProvidersSection(model: AppModel, deviceId: String) {
    val scope = rememberCoroutineScope()
    var providers by remember(deviceId) { mutableStateOf<List<PiProviders.Provider>?>(null) }
    var error by remember(deviceId) { mutableStateOf<String?>(null) }
    var unsupported by remember(deviceId) { mutableStateOf(false) }
    // `null` closed; `Provider?` inside: edit that one, or add (null).
    var editing by remember { mutableStateOf<Pair<Boolean, PiProviders.Provider?>?>(null) }
    var menuFor by remember { mutableStateOf<String?>(null) }
    var confirmRemove by remember { mutableStateOf<PiProviders.Provider?>(null) }

    suspend fun load() {
        runCatching { PiProviders.providers(model.hostCall(deviceId, PiProviders.LIST)) }
            .onSuccess { providers = it; error = null }
            .onFailure { if (it is CoreException.Unsupported) unsupported = true else error = it.userMessage() }
    }
    LaunchedEffect(deviceId) { load() }
    if (unsupported) return

    Column(Modifier.padding(horizontal = 16.dp)) {
        Text(
            "Custom providers",
            style = MaterialTheme.typography.titleSmallEmphasized,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
        )
        val list = providers
        when {
            list == null && error != null -> Text(error!!, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 12.dp))
            list == null -> Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) { LoadingIndicator(Modifier.size(32.dp)) }
            else -> Column(verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                val total = list.size + 1
                list.forEachIndexed { i, p ->
                    Box {
                        SegmentedListItem(
                            onClick = { editing = true to p },
                            onLongClick = { menuFor = p.name },
                            shapes = segmentedShapes(i, total),
                            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                            leadingContent = { IconTile(ZIcons.Server) },
                            supportingContent = {
                                val count = if (p.models.size == 1) "1 model" else "${p.models.size} models"
                                Text("${p.host} · $count · ${p.keyLabel}", maxLines = 1, overflow = TextOverflow.Ellipsis)
                            },
                        ) { Text(p.name, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                        ActionMenu(
                            menuFor == p.name,
                            { menuFor = null },
                            listOf(
                                MenuAction("Edit", ZIcons.Rename) { editing = true to p },
                                MenuAction("Remove", ZIcons.Delete, destructive = true) { confirmRemove = p },
                            ),
                        )
                    }
                }
                SegmentedListItem(
                    onClick = { editing = true to null },
                    shapes = segmentedShapes(list.size, total),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Plus) },
                ) { Text("Add custom provider") }
            }
        }
        Text(
            "OpenAI-compatible endpoints Pi can use. Their models show up in Pi's model picker.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
        )
    }

    editing?.let { (_, existing) ->
        PiProviderDialog(model, deviceId, existing, onDismiss = { editing = null }, onSaved = {
            providers = it
            editing = null
        })
    }
    confirmRemove?.let { p ->
        AlertDialog(
            onDismissRequest = { confirmRemove = null },
            icon = { ZIcon(ZIcons.Delete, null) },
            title = { Text("Remove ${p.name}?") },
            text = { Text("Its entry, key and models are deleted from Pi's models.json on this device.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = null
                    scope.launch {
                        runCatching { PiProviders.providers(model.hostCall(deviceId, PiProviders.REMOVE, JSONObject().put("name", p.name))) }
                            .onSuccess { providers = it }
                            .onFailure { model.toast(it.userMessage()) }
                    }
                }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmRemove = null }) { Text("Cancel") } },
        )
    }
}

/** Add (`existing` = null) or edit one provider. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun PiProviderDialog(
    model: AppModel,
    deviceId: String,
    existing: PiProviders.Provider?,
    onDismiss: () -> Unit,
    onSaved: (List<PiProviders.Provider>) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf(existing?.name ?: "") }
    var baseUrl by remember { mutableStateOf(existing?.baseUrl ?: "https://") }
    var apiKey by remember { mutableStateOf("") }
    var api by remember { mutableStateOf(existing?.api ?: PiProviders.APIS.first().first) }
    val selected = remember { mutableStateListOf<String>().apply { existing?.models?.forEach { add(it.id) } } }
    // Names a kept model already had (Pi shows them in the picker).
    val names = remember { existing?.models?.mapNotNull { m -> m.name?.let { m.id to it } }?.toMap().orEmpty() }
    var fetched by remember { mutableStateOf<List<String>>(emptyList()) }
    var filter by remember { mutableStateOf("") }
    var manual by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    fun fetch() {
        busy = "fetch"
        error = null
        scope.launch {
            val params = JSONObject().put("baseUrl", baseUrl.trim())
            if (apiKey.isNotBlank()) params.put("apiKey", apiKey.trim())
            existing?.let { params.put("name", it.name) }
            runCatching { PiProviders.discovered(model.hostCall(deviceId, PiProviders.DISCOVER, params)) }
                .onSuccess { ids ->
                    fetched = ids
                    if (ids.isEmpty()) error = "The endpoint listed no models — add them by id below."
                }
                .onFailure { error = it.userMessage() }
            busy = null
        }
    }

    fun save() {
        busy = "save"
        error = null
        scope.launch {
            val models = JSONArray()
            selected.forEach { id -> models.put(JSONObject().put("id", id).apply { names[id]?.let { put("name", it) } }) }
            val params = JSONObject()
                .put("name", name.trim())
                .put("baseUrl", baseUrl.trim())
                .put("api", api)
                .put("models", models)
            existing?.let { params.put("previousName", it.name) }
            if (apiKey.isNotBlank()) params.put("apiKey", apiKey.trim())
            runCatching { PiProviders.providers(model.hostCall(deviceId, PiProviders.SAVE, params)) }
                .onSuccess(onSaved)
                .onFailure { error = it.userMessage() }
            busy = null
        }
    }

    AlertDialog(
        onDismissRequest = { if (busy == null) onDismiss() },
        icon = { ZIcon(ZIcons.Server, null) },
        title = { Text(if (existing == null) "Add custom provider" else "Edit ${existing.name}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = name, onValueChange = { name = it.lowercase().replace(' ', '-') },
                    label = { Text("Name") }, placeholder = { Text("omni") },
                    singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = baseUrl, onValueChange = { baseUrl = it },
                    label = { Text("Base URL") }, placeholder = { Text("https://api.example.com/v1") },
                    singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = apiKey, onValueChange = { apiKey = it },
                    label = { Text("API key") },
                    placeholder = { Text(if (existing != null && existing.keySource != "none") "Unchanged (${existing.keyLabel})" else "sk-…") },
                    singleLine = true, visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    val choices = PiProviders.APIS.toMutableList()
                    if (choices.none { it.first == api }) choices.add(api to api)
                    choices.forEach { (id, label) ->
                        FilterChip(selected = api == id, onClick = { api = id }, label = { Text(label) })
                    }
                }

                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        if (selected.size == 1) "1 model selected" else "${selected.size} models selected",
                        style = MaterialTheme.typography.titleSmall,
                        modifier = Modifier.weight(1f),
                    )
                    if (busy == "fetch") LoadingIndicator(Modifier.size(28.dp))
                    else TextButton(onClick = ::fetch, enabled = busy == null) { Text("Fetch models") }
                }
                if (fetched.isNotEmpty()) {
                    OutlinedTextField(
                        value = filter, onValueChange = { filter = it },
                        label = { Text("Search ${fetched.size} models") }, singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                // Selected first, then the fetched rest, narrowed by the search.
                val rows = (selected + fetched.filterNot { it in selected })
                    .filter { filter.isBlank() || it.contains(filter.trim(), ignoreCase = true) }
                if (rows.isNotEmpty()) {
                    LazyColumn(Modifier.fillMaxWidth().heightIn(max = 240.dp)) {
                        items(rows, key = { it }) { id ->
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Checkbox(checked = id in selected, onCheckedChange = { on -> if (on) selected.add(id) else selected.remove(id) })
                                Text(id, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = manual, onValueChange = { manual = it },
                        label = { Text("Add model by id") }, singleLine = true,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(
                        onClick = {
                            val id = manual.trim()
                            if (id.isNotEmpty() && id !in selected) selected.add(id)
                            manual = ""
                        },
                        enabled = manual.isNotBlank(),
                    ) { Text("Add") }
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = {
            if (busy == "save") LoadingIndicator(Modifier.size(32.dp))
            else TextButton(
                onClick = ::save,
                enabled = busy == null && name.isNotBlank() && baseUrl.isNotBlank() && selected.isNotEmpty(),
            ) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = busy == null) { Text("Cancel") } },
    )
}
