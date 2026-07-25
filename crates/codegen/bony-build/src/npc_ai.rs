//! Unity NPC AI runtime scripts installed into the open project via CLI eval.
//! Play Mode: approach NPC, press E, chat via xAI (`XAI_API_KEY`) with offline fallback.

/// C# source for `Assets/Bony/NpcAi/BonyNpcBrain.cs`
pub const SCRIPT_NPC_BRAIN: &str = r#"using UnityEngine;

/// Persona + role for an AI-backed NPC (installed by Bony Build).
public class BonyNpcBrain : MonoBehaviour
{
    public string displayName = "NPC";
    public string role = "villager";
    [TextArea(3, 8)]
    public string persona = "You are a friendly townsperson in a small fantasy village. Stay in character. Keep replies short (1-2 sentences).";
    public string model = "grok-2-latest";
}
"#;

/// C# source for `Assets/Bony/NpcAi/BonyNpcDialogue.cs`
pub const SCRIPT_NPC_DIALOGUE: &str = r#"using System.Collections;
using System.Text;
using UnityEngine;
using UnityEngine.Networking;

/// Proximity dialogue + xAI chat for NPCs that have BonyNpcBrain.
public class BonyNpcDialogue : MonoBehaviour
{
    public float interactRange = 3f;
    public KeyCode interactKey = KeyCode.E;

    BonyNpcBrain brain;
    Transform player;
    bool open;
    bool busy;
    string input = "";
    string log = "";

    [System.Serializable] class ChatMsg { public string role; public string content; }
    [System.Serializable] class ChatReq { public string model; public ChatMsg[] messages; public float temperature; }
    [System.Serializable] class ChoiceMsg { public string role; public string content; }
    [System.Serializable] class Choice { public ChoiceMsg message; }
    [System.Serializable] class ChatResp { public Choice[] choices; }

    void Awake()
    {
        brain = GetComponent<BonyNpcBrain>();
        if (brain == null) brain = gameObject.AddComponent<BonyNpcBrain>();
    }

    void Update()
    {
        if (player == null)
        {
            var p = GameObject.Find("Player");
            if (p != null) player = p.transform;
        }
        if (player == null) return;
        float d = Vector3.Distance(transform.position, player.position);
        if (d <= interactRange && Input.GetKeyDown(interactKey))
        {
            open = !open;
            if (open && string.IsNullOrEmpty(log))
                log = (brain != null ? brain.displayName : name) + ": 你好，有什么事吗？\n";
        }
        if (open && d > interactRange + 0.75f) open = false;
    }

    void OnGUI()
    {
        if (player != null && !open && Vector3.Distance(transform.position, player.position) <= interactRange)
        {
            var cam = Camera.main;
            if (cam != null)
            {
                var sp = cam.WorldToScreenPoint(transform.position + Vector3.up * 2f);
                if (sp.z > 0f)
                    GUI.Label(new Rect(sp.x - 48f, Screen.height - sp.y - 18f, 140f, 24f), "[E] 对话");
            }
        }
        if (!open) return;
        float w = 440f, h = 280f;
        var title = brain != null ? brain.displayName : name;
        GUI.Window(GetInstanceID(), new Rect((Screen.width - w) * 0.5f, Screen.height - h - 28f, w, h), DrawWindow, title);
    }

    void DrawWindow(int id)
    {
        GUILayout.Label(log, GUILayout.Height(170f));
        GUILayout.BeginHorizontal();
        GUI.enabled = !busy;
        input = GUILayout.TextField(input);
        if (GUILayout.Button(busy ? "…" : "发送", GUILayout.Width(64f)))
            TrySend();
        GUI.enabled = true;
        GUILayout.EndHorizontal();
        if (Event.current.type == EventType.KeyDown && Event.current.keyCode == KeyCode.Return)
            TrySend();
        if (GUILayout.Button("关闭")) open = false;
        GUI.DragWindow();
    }

    void TrySend()
    {
        if (busy || string.IsNullOrWhiteSpace(input)) return;
        var text = input.Trim();
        input = "";
        StartCoroutine(AskAi(text));
    }

    static string ResolveApiKey()
    {
        var env = System.Environment.GetEnvironmentVariable("XAI_API_KEY");
        if (!string.IsNullOrEmpty(env)) return env;
        if (PlayerPrefs.HasKey("BonyXaiApiKey"))
            return PlayerPrefs.GetString("BonyXaiApiKey");
        return "";
    }

    IEnumerator AskAi(string userText)
    {
        busy = true;
        var who = brain != null ? brain.displayName : name;
        log += "你: " + userText + "\n";
        var key = ResolveApiKey();
        if (string.IsNullOrEmpty(key))
        {
            log += who + ": （未配置 XAI_API_KEY，离线回复）我是本地的" + (brain != null ? brain.role : "npc") + "，以后配好 Key 就能真正对话了。\n";
            busy = false;
            yield break;
        }

        var persona = brain != null ? brain.persona : "You are a helpful NPC.";
        var role = brain != null ? brain.role : "villager";
        var model = brain != null ? brain.model : "grok-2-latest";
        var sys = persona + " Your in-game role is " + role + ". Reply in the same language as the player. Keep answers under two short sentences.";
        var req = new ChatReq
        {
            model = model,
            temperature = 0.7f,
            messages = new[]
            {
                new ChatMsg { role = "system", content = sys },
                new ChatMsg { role = "user", content = userText }
            }
        };
        var json = JsonUtility.ToJson(req);
        using (var uwr = new UnityWebRequest("https://api.x.ai/v1/chat/completions", "POST"))
        {
            uwr.uploadHandler = new UploadHandlerRaw(Encoding.UTF8.GetBytes(json));
            uwr.downloadHandler = new DownloadHandlerBuffer();
            uwr.SetRequestHeader("Content-Type", "application/json");
            uwr.SetRequestHeader("Authorization", "Bearer " + key);
            yield return uwr.SendWebRequest();
#if UNITY_2020_2_OR_NEWER
            bool failed = uwr.result != UnityWebRequest.Result.Success;
#else
            bool failed = uwr.isNetworkError || uwr.isHttpError;
#endif
            if (failed)
            {
                log += who + ": （AI 请求失败）" + uwr.error + "\n";
            }
            else
            {
                var resp = JsonUtility.FromJson<ChatResp>(uwr.downloadHandler.text);
                var reply = (resp != null && resp.choices != null && resp.choices.Length > 0 && resp.choices[0].message != null)
                    ? resp.choices[0].message.content
                    : "";
                if (string.IsNullOrEmpty(reply))
                    log += who + ": （空回复）\n";
                else
                    log += who + ": " + reply.Trim() + "\n";
            }
        }
        busy = false;
    }
}
"#;

fn csharp_escape(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "")
        .replace('\n', "\\n")
}

/// Writes both NPC AI scripts under `Assets/Bony/NpcAi/` and refreshes the AssetDatabase.
pub fn eval_install_npc_ai_scripts() -> String {
    let brain = csharp_escape(SCRIPT_NPC_BRAIN);
    let dialogue = csharp_escape(SCRIPT_NPC_DIALOGUE);
    format!(
        r#"var root = System.IO.Path.Combine(UnityEngine.Application.dataPath, "Bony", "NpcAi"); if (!System.IO.Directory.Exists(root)) System.IO.Directory.CreateDirectory(root); System.IO.File.WriteAllText(System.IO.Path.Combine(root, "BonyNpcBrain.cs"), "{brain}"); System.IO.File.WriteAllText(System.IO.Path.Combine(root, "BonyNpcDialogue.cs"), "{dialogue}"); UnityEditor.AssetDatabase.Refresh(); return "installed Assets/Bony/NpcAi (BonyNpcBrain + BonyNpcDialogue)";"#
    )
}

/// Attaches brain + dialogue to every scene object whose name starts with `NPC_`.
pub const EVAL_ATTACH_NPC_AI: &str = r#"System.Type FindType(string name) { foreach (var asm in System.AppDomain.CurrentDomain.GetAssemblies()) { var t = asm.GetType(name); if (t != null) return t; } return null; } var brainT = FindType("BonyNpcBrain"); var talkT = FindType("BonyNpcDialogue"); if (brainT == null || talkT == null) return "NPC AI scripts not compiled yet — wait for reload, then Attach again"; int n = 0; foreach (var go in UnityEngine.Object.FindObjectsByType<UnityEngine.GameObject>(UnityEngine.FindObjectsSortMode.None)) { if (go == null || !go.name.StartsWith("NPC_")) continue; var brain = go.GetComponent(brainT) ?? UnityEditor.Undo.AddComponent(go, brainT); var talk = go.GetComponent(talkT) ?? UnityEditor.Undo.AddComponent(go, talkT); var display = brainT.GetField("displayName"); var role = brainT.GetField("role"); var persona = brainT.GetField("persona"); if (display != null) display.SetValue(brain, go.name); if (go.name.Contains("Vendor")) { if (role != null) role.SetValue(brain, "vendor"); if (persona != null) persona.SetValue(brain, "You are a cheerful market vendor in a fantasy town. Talk about wares and prices. Keep replies short."); if (display != null) display.SetValue(brain, "商人"); } else if (go.name.Contains("Quest")) { if (role != null) role.SetValue(brain, "quest_giver"); if (persona != null) persona.SetValue(brain, "You are a quest giver. Hint at simple tasks without inventing complex systems. Keep replies short."); if (display != null) display.SetValue(brain, "任务人"); } UnityEditor.EditorUtility.SetDirty(go); n++; } UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(UnityEngine.SceneManagement.SceneManager.GetActiveScene()); return "attached NPC AI to " + n + " objects";"#;
