package ai.kratos.tailcat

import android.os.Build
import android.util.Base64
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject
import tailcatnative.Client
import tailcatnative.Tailcatnative

class KratosTailcatModule : Module() {
  private var client: Client? = null

  override fun definition() = ModuleDefinition {
    Name("KratosTailcat")

    AsyncFunction("connect") { invitation: String ->
      val payload = invitation.trim().removePrefix("kratos-pair:")
      val json = if (payload.startsWith('{')) payload else String(
        Base64.decode(payload, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP), Charsets.UTF_8
      )
      require(json.length <= 16 * 1024) { "Invitation is too large" }
      val invite = JSONObject(json)
      require(invite.optInt("version") == 1) { "Unsupported invitation" }
      val address = invite.optString("address")
      require(address.isNotBlank()) { "Invitation has no Tailcat address" }
      val stateDirectory = requireNotNull(appContext.reactContext).filesDir.resolve("tailcat")
      stateDirectory.mkdirs()
      client?.close()
      client = null
      val next = start(address, stateDirectory.absolutePath, invite.optString("derpMap"))
      try {
        val deviceName = Build.MODEL.trim().filterNot { Character.isISOControl(it) }.take(32).ifEmpty { "Android" }
        val session = JSONObject(next.pair(json, deviceName))
        val saved = JSONObject().put("address", address)
          .put("derpMap", invite.optString("derpMap"))
          .put("profileId", session.getJSONObject("principal").getString("profileId"))
        check(preferences().edit().putString("connection", saved.toString()).commit()) {
          "Could not save device pairing"
        }
        client = next
        session.put("baseUrl", next.url()).toString()
      } catch (error: Exception) {
        next.close()
        throw error
      }
    }

    AsyncFunction("restore") {
      val saved = preferences().getString("connection", null) ?: return@AsyncFunction null
      val connection = JSONObject(saved)
      val stateDirectory = requireNotNull(appContext.reactContext).filesDir.resolve("tailcat")
      client?.close()
      client = null
      val next = start(connection.getString("address"), stateDirectory.absolutePath, connection.optString("derpMap"))
      try {
        val session = JSONObject(next.authenticate(connection.getString("profileId")))
        client = next
        session.put("baseUrl", next.url()).toString()
      } catch (error: Exception) {
        next.close()
        throw error
      }
    }

    AsyncFunction("renew") {
      val profileId = JSONObject(preferences().getString("connection", null)
        ?: error("Device is not paired")).getString("profileId")
      val current = client ?: error("Tailcat is disconnected")
      current.authenticate(profileId)
    }

    Function("disconnect") {
      client?.close()
      client = null
    }

    OnDestroy {
      client?.close()
      client = null
    }
  }

  private fun start(address: String, directory: String, derpMap: String): Client =
    if (BuildConfig.DEBUG) Tailcatnative.startClientDiagnostic(address, directory, derpMap)
    else Tailcatnative.startClient(address, directory, derpMap)

  private fun preferences() = requireNotNull(appContext.reactContext)
    .getSharedPreferences("kratos-connection", android.content.Context.MODE_PRIVATE)
}
