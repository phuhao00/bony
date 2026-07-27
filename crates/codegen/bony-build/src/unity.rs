//! Unity CLI bridge for desktop visualization.
//!
//! Wraps the standalone [Unity CLI](https://unity.com/cn/blog/meet-the-unity-cli)
//! (`unity`) so the agent desktop can detect the binary, run structured
//! commands, and show an observe → act → verify feedback loop.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INSTALL_LOG_TAIL_MAX: usize = 60;

const CMD_TIMEOUT: Duration = Duration::from_secs(45);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
const TEST_TIMEOUT: Duration = Duration::from_secs(600);
const BUILD_TIMEOUT: Duration = Duration::from_secs(1800);
const PACKAGE_TIMEOUT: Duration = Duration::from_secs(300);
const GUIDE_STEP_GAP: Duration = Duration::from_millis(650);

const EVAL_SAVE_SCENE: &str =
    "return UnityEditor.SceneManagement.EditorSceneManager.SaveOpenScenes();";
const EVAL_REFRESH_ASSETS: &str = "UnityEditor.AssetDatabase.Refresh(); return true;";
const EVAL_SCRIPT_RELOAD: &str = "UnityEditor.EditorUtility.RequestScriptReload(); return true;";
const EVAL_CLEAR_CONSOLE: &str = "var asm = System.Reflection.Assembly.GetAssembly(typeof(UnityEditor.Editor)); var t = asm.GetType(\"UnityEditor.LogEntries\"); t.GetMethod(\"Clear\").Invoke(null, null); return \"cleared\";";
const EVAL_PAUSE_PLAY: &str = "UnityEditor.EditorApplication.isPaused = !UnityEditor.EditorApplication.isPaused; return UnityEditor.EditorApplication.isPaused;";
const EVAL_STEP_PLAY: &str = "UnityEditor.EditorApplication.Step(); return true;";
const EVAL_UNDO: &str = "UnityEditor.Undo.PerformUndo(); return \"undo\";";
const EVAL_REDO: &str = "UnityEditor.Undo.PerformRedo(); return \"redo\";";
const EVAL_FRAME_SELECTION: &str = "UnityEditor.SceneView.FrameLastActiveSceneView(); return UnityEditor.Selection.activeGameObject != null ? UnityEditor.Selection.activeGameObject.name : \"none\";";
const EVAL_FOCUS_GAME: &str = "UnityEditor.EditorApplication.ExecuteMenuItem(\"Window/General/Game\"); return \"Game\";";
const EVAL_FOCUS_SCENE: &str = "UnityEditor.EditorApplication.ExecuteMenuItem(\"Window/General/Scene\"); return \"Scene\";";
const EVAL_DUPLICATE_SELECTION: &str = "UnityEditor.Unsupported.DuplicateGameObjectsUsingPasteboard(); return UnityEditor.Selection.gameObjects.Length;";
const EVAL_DELETE_SELECTION: &str = "var objs = UnityEditor.Selection.gameObjects; if (objs == null || objs.Length == 0) return 0; UnityEditor.Undo.DestroyObjectImmediate(objs[0]); for (int i = 1; i < objs.Length; i++) UnityEditor.Undo.DestroyObjectImmediate(objs[i]); return objs.Length;";
const EVAL_CREATE_CUBE: &str = "var go = GameObject.CreatePrimitive(PrimitiveType.Cube); go.name = \"BonyCube\"; go.transform.position = Vector3.zero; UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_CAPSULE: &str = "var go = GameObject.CreatePrimitive(PrimitiveType.Capsule); go.name = \"BonyCapsule\"; go.transform.position = Vector3.zero; UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_PLANE: &str = "var go = GameObject.CreatePrimitive(PrimitiveType.Plane); go.name = \"BonyPlane\"; go.transform.position = Vector3.zero; UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_LIGHT: &str = "var go = new GameObject(\"BonyDirectionalLight\"); var light = go.AddComponent<Light>(); light.type = LightType.Directional; light.intensity = 1f; go.transform.rotation = Quaternion.Euler(50f, -30f, 0f); UnityEditor.Undo.RegisterCreatedObjectUndo(go, \"Create light\"); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_SETUP_SKY_DAY: &str = "RenderSettings.ambientMode = UnityEngine.Rendering.AmbientMode.Trilight; RenderSettings.ambientSkyColor = new Color(0.45f, 0.65f, 0.95f); RenderSettings.ambientEquatorColor = new Color(0.55f, 0.6f, 0.65f); RenderSettings.ambientGroundColor = new Color(0.25f, 0.22f, 0.2f); var shader = Shader.Find(\"Skybox/Procedural\"); if (shader != null) { var mat = new Material(shader); mat.name = \"BonySkyDay\"; if (mat.HasProperty(\"_SkyTint\")) mat.SetColor(\"_SkyTint\", new Color(0.4f, 0.55f, 0.9f)); if (mat.HasProperty(\"_AtmosphereThickness\")) mat.SetFloat(\"_AtmosphereThickness\", 1.1f); if (mat.HasProperty(\"_Exposure\")) mat.SetFloat(\"_Exposure\", 1.2f); RenderSettings.skybox = mat; } else { RenderSettings.skybox = null; } var cam = Camera.main; if (cam != null) { cam.clearFlags = CameraClearFlags.Skybox; cam.backgroundColor = new Color(0.45f, 0.7f, 1f); } DynamicGI.UpdateEnvironment(); return \"day\";";
const EVAL_SETUP_SKY_SUNSET: &str = "RenderSettings.ambientMode = UnityEngine.Rendering.AmbientMode.Trilight; RenderSettings.ambientSkyColor = new Color(0.95f, 0.45f, 0.25f); RenderSettings.ambientEquatorColor = new Color(0.85f, 0.4f, 0.35f); RenderSettings.ambientGroundColor = new Color(0.2f, 0.12f, 0.1f); var shader = Shader.Find(\"Skybox/Procedural\"); if (shader != null) { var mat = new Material(shader); mat.name = \"BonySkySunset\"; if (mat.HasProperty(\"_SkyTint\")) mat.SetColor(\"_SkyTint\", new Color(0.95f, 0.4f, 0.2f)); if (mat.HasProperty(\"_GroundColor\")) mat.SetColor(\"_GroundColor\", new Color(0.35f, 0.15f, 0.1f)); if (mat.HasProperty(\"_AtmosphereThickness\")) mat.SetFloat(\"_AtmosphereThickness\", 1.4f); if (mat.HasProperty(\"_Exposure\")) mat.SetFloat(\"_Exposure\", 1.0f); RenderSettings.skybox = mat; } else { RenderSettings.skybox = null; } var cam = Camera.main; if (cam != null) { cam.clearFlags = CameraClearFlags.Skybox; cam.backgroundColor = new Color(0.95f, 0.5f, 0.3f); } DynamicGI.UpdateEnvironment(); return \"sunset\";";
const EVAL_SETUP_SKY_NIGHT: &str = "RenderSettings.ambientMode = UnityEngine.Rendering.AmbientMode.Trilight; RenderSettings.ambientSkyColor = new Color(0.05f, 0.08f, 0.18f); RenderSettings.ambientEquatorColor = new Color(0.08f, 0.1f, 0.18f); RenderSettings.ambientGroundColor = new Color(0.02f, 0.02f, 0.04f); var shader = Shader.Find(\"Skybox/Procedural\"); if (shader != null) { var mat = new Material(shader); mat.name = \"BonySkyNight\"; if (mat.HasProperty(\"_SkyTint\")) mat.SetColor(\"_SkyTint\", new Color(0.05f, 0.08f, 0.2f)); if (mat.HasProperty(\"_AtmosphereThickness\")) mat.SetFloat(\"_AtmosphereThickness\", 0.6f); if (mat.HasProperty(\"_Exposure\")) mat.SetFloat(\"_Exposure\", 0.35f); RenderSettings.skybox = mat; } else { RenderSettings.skybox = null; } var cam = Camera.main; if (cam != null) { cam.clearFlags = CameraClearFlags.Skybox; cam.backgroundColor = new Color(0.02f, 0.03f, 0.08f); } DynamicGI.UpdateEnvironment(); return \"night\";";
const EVAL_CREATE_GROUND: &str = "var go = GameObject.Find(\"Ground\"); if (go == null) { go = GameObject.CreatePrimitive(PrimitiveType.Plane); go.name = \"Ground\"; UnityEditor.Undo.RegisterCreatedObjectUndo(go, \"Create Ground\"); } go.transform.position = Vector3.zero; go.transform.localScale = new Vector3(8f, 1f, 8f); var col = go.GetComponent<Collider>(); if (col != null) col.enabled = true; UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name + \" scale=\" + go.transform.localScale;";
const EVAL_SETUP_MAIN_CAMERA: &str = "var cam = Camera.main; GameObject cgo; if (cam == null) { cgo = new GameObject(\"Main Camera\"); cam = cgo.AddComponent<Camera>(); cgo.tag = \"MainCamera\"; if (cgo.GetComponent<AudioListener>() == null) cgo.AddComponent<AudioListener>(); UnityEditor.Undo.RegisterCreatedObjectUndo(cgo, \"Create Main Camera\"); } else { cgo = cam.gameObject; } UnityEditor.Undo.RecordObject(cgo.transform, \"Frame Main Camera\"); cgo.transform.position = new Vector3(0f, 5f, -10f); cgo.transform.rotation = Quaternion.Euler(20f, 0f, 0f); cam.clearFlags = CameraClearFlags.Skybox; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(cgo.scene); return cgo.name + \" @ \" + cgo.transform.position;";
const EVAL_CREATE_PLAYER: &str = "var go = GameObject.Find(\"Player\"); if (go == null) { go = GameObject.CreatePrimitive(PrimitiveType.Capsule); go.name = \"Player\"; UnityEditor.Undo.RegisterCreatedObjectUndo(go, \"Create Player\"); } go.transform.position = new Vector3(0f, 1.1f, 0f); go.transform.localScale = Vector3.one; var rb = go.GetComponent<Rigidbody>(); if (rb == null) rb = UnityEditor.Undo.AddComponent<Rigidbody>(go); rb.constraints = RigidbodyConstraints.FreezeRotation; rb.collisionDetectionMode = CollisionDetectionMode.Continuous; UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name + \" y=\" + go.transform.position.y;";
const EVAL_CREATE_NPC: &str = "int i = 1; string name; do { name = \"NPC_\" + i; i++; } while (GameObject.Find(name) != null); var go = GameObject.CreatePrimitive(PrimitiveType.Capsule); go.name = name; go.transform.position = new Vector3((i - 2) * 2f, 0.5f, 2f); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_NPC_VENDOR: &str = "var go = GameObject.Find(\"NPC_Vendor\"); if (go == null) { go = GameObject.CreatePrimitive(PrimitiveType.Capsule); go.name = \"NPC_Vendor\"; UnityEditor.Undo.RegisterCreatedObjectUndo(go, \"NPC_Vendor\"); } go.transform.position = new Vector3(4f, 0.5f, 2f); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_NPC_QUEST: &str = "var go = GameObject.Find(\"NPC_Quest\"); if (go == null) { go = GameObject.CreatePrimitive(PrimitiveType.Capsule); go.name = \"NPC_Quest\"; UnityEditor.Undo.RegisterCreatedObjectUndo(go, \"NPC_Quest\"); } go.transform.position = new Vector3(-4f, 0.5f, 2f); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_SPAWN_POINT: &str = "int i = 1; string name; do { name = \"Spawn_\" + i; i++; } while (GameObject.Find(name) != null); var go = new GameObject(name); go.transform.position = new Vector3((i - 2) * 3f, 0.1f, -6f); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_PORTAL_ZONE: &str = "int i = 1; string name; do { name = i == 1 ? \"Portal_Zone\" : (\"Portal_Zone_\" + i); i++; } while (GameObject.Find(name) != null); var go = GameObject.CreatePrimitive(PrimitiveType.Cylinder); go.name = name; go.transform.position = new Vector3(0f, 1f, 8f + (i - 2) * 3f); go.transform.localScale = new Vector3(2f, 2f, 2f); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_CREATE_ENEMY_SPAWN: &str = "int i = 1; string name; do { name = \"Enemy_Spawn_\" + i; i++; } while (GameObject.Find(name) != null); var go = new GameObject(name); go.transform.position = new Vector3((i - 2) * 4f, 0.5f, 6f); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";
const EVAL_LAYOUT_RPG: &str = "System.Func<string,Vector3,PrimitiveType,GameObject> prim = (name, pos, t) => { var go = GameObject.Find(name); if (go == null) { go = GameObject.CreatePrimitive(t); go.name = name; UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); } go.transform.position = pos; return go; }; System.Func<string,Vector3,GameObject> empty = (name, pos) => { var go = GameObject.Find(name); if (go == null) { go = new GameObject(name); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); } go.transform.position = pos; return go; }; var ground = prim(\"Ground\", Vector3.zero, PrimitiveType.Plane); ground.transform.localScale = new Vector3(10f,1f,10f); prim(\"NPC_Vendor\", new Vector3(4f,0.5f,2f), PrimitiveType.Capsule); prim(\"NPC_Quest\", new Vector3(-4f,0.5f,2f), PrimitiveType.Capsule); empty(\"Spawn_Town\", new Vector3(0f,0.1f,-6f)); var player = prim(\"Player\", new Vector3(0f,1.1f,-4f), PrimitiveType.Capsule); var rb = player.GetComponent<Rigidbody>() ?? UnityEditor.Undo.AddComponent<Rigidbody>(player); rb.constraints = RigidbodyConstraints.FreezeRotation; var canvasGo = GameObject.Find(\"Canvas\"); if (canvasGo == null) { canvasGo = new GameObject(\"Canvas\"); var c = canvasGo.AddComponent<Canvas>(); c.renderMode = RenderMode.ScreenSpaceOverlay; canvasGo.AddComponent<UnityEngine.UI.CanvasScaler>(); canvasGo.AddComponent<UnityEngine.UI.GraphicRaycaster>(); UnityEditor.Undo.RegisterCreatedObjectUndo(canvasGo, \"Canvas\"); } var hud = GameObject.Find(\"HUD\"); if (hud == null) { hud = new GameObject(\"HUD\"); hud.transform.SetParent(canvasGo.transform, false); UnityEditor.Undo.RegisterCreatedObjectUndo(hud, \"HUD\"); } System.Action<string,string,Vector2> label = (n, text, anchored) => { var go = GameObject.Find(n); if (go == null) { go = new GameObject(n); go.transform.SetParent(hud.transform, false); var t = go.AddComponent<UnityEngine.UI.Text>(); t.font = Resources.GetBuiltinResource<Font>(\"Arial.ttf\"); t.text = text; t.fontSize = 22; t.color = Color.white; var rt = go.GetComponent<RectTransform>(); rt.anchorMin = new Vector2(0,1); rt.anchorMax = new Vector2(0,1); rt.pivot = new Vector2(0,1); rt.anchoredPosition = anchored; rt.sizeDelta = new Vector2(240, 36); UnityEditor.Undo.RegisterCreatedObjectUndo(go, n); } }; label(\"HUD_HP\", \"HP 100/100\", new Vector2(16,-16)); label(\"HUD_Gold\", \"Gold 0\", new Vector2(16,-56)); UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(UnityEngine.SceneManagement.SceneManager.GetActiveScene()); return \"rpg layout: NPC_Vendor NPC_Quest Spawn_Town HUD\";";
const EVAL_LAYOUT_MMO: &str = "System.Func<string,Vector3,PrimitiveType,GameObject> prim = (name, pos, t) => { var go = GameObject.Find(name); if (go == null) { go = GameObject.CreatePrimitive(t); go.name = name; UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); } go.transform.position = pos; return go; }; System.Func<string,Vector3,GameObject> empty = (name, pos) => { var go = GameObject.Find(name); if (go == null) { go = new GameObject(name); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); } go.transform.position = pos; return go; }; var hub = prim(\"World_Hub\", Vector3.zero, PrimitiveType.Plane); hub.transform.localScale = new Vector3(20f,1f,20f); empty(\"Spawn_A\", new Vector3(-8f,0.2f,-8f)); empty(\"Spawn_B\", new Vector3(0f,0.2f,-10f)); empty(\"Spawn_C\", new Vector3(8f,0.2f,-8f)); var portal = prim(\"Portal_Zone\", new Vector3(0f,1f,10f), PrimitiveType.Cylinder); portal.transform.localScale = new Vector3(2f,2f,2f); var player = prim(\"Player\", new Vector3(0f,1.1f,-6f), PrimitiveType.Capsule); var rb = player.GetComponent<Rigidbody>() ?? UnityEditor.Undo.AddComponent<Rigidbody>(player); rb.constraints = RigidbodyConstraints.FreezeRotation; var canvasGo = GameObject.Find(\"Canvas\"); if (canvasGo == null) { canvasGo = new GameObject(\"Canvas\"); var c = canvasGo.AddComponent<Canvas>(); c.renderMode = RenderMode.ScreenSpaceOverlay; canvasGo.AddComponent<UnityEngine.UI.CanvasScaler>(); canvasGo.AddComponent<UnityEngine.UI.GraphicRaycaster>(); UnityEditor.Undo.RegisterCreatedObjectUndo(canvasGo, \"Canvas\"); } System.Action<string,Vector2,Vector2> panel = (n, anchor, size) => { var go = GameObject.Find(n); if (go == null) { go = new GameObject(n); go.transform.SetParent(canvasGo.transform, false); var img = go.AddComponent<UnityEngine.UI.Image>(); img.color = new Color(0f,0f,0f,0.45f); var rt = go.GetComponent<RectTransform>(); rt.anchorMin = anchor; rt.anchorMax = anchor; rt.pivot = anchor; rt.anchoredPosition = Vector2.zero; rt.sizeDelta = size; UnityEditor.Undo.RegisterCreatedObjectUndo(go, n); } }; panel(\"ChatPanel\", new Vector2(0,0), new Vector2(360,180)); panel(\"MinimapFrame\", new Vector2(1,1), new Vector2(160,160)); UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(UnityEngine.SceneManagement.SceneManager.GetActiveScene()); return \"mmo layout: World_Hub Spawn_A Spawn_B Spawn_C Portal_Zone ChatPanel MinimapFrame\";";
const EVAL_LAYOUT_ROGUELIKE: &str = "System.Func<string,Vector3,Vector3,GameObject> room = (name, pos, scale) => { var go = GameObject.Find(name); if (go == null) { go = GameObject.CreatePrimitive(PrimitiveType.Cube); go.name = name; UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); } go.transform.position = pos; go.transform.localScale = scale; return go; }; System.Func<string,Vector3,GameObject> empty = (name, pos) => { var go = GameObject.Find(name); if (go == null) { go = new GameObject(name); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); } go.transform.position = pos; return go; }; int idx = 0; for (int z = -1; z <= 1; z++) { for (int x = -1; x <= 1; x++) { room(\"Room_\" + idx, new Vector3(x * 8f, -0.5f, z * 8f), new Vector3(6f, 0.2f, 6f)); idx++; } } empty(\"Enemy_Spawn_1\", new Vector3(8f, 0.5f, 0f)); empty(\"Enemy_Spawn_2\", new Vector3(-8f, 0.5f, 0f)); empty(\"Enemy_Spawn_3\", new Vector3(0f, 0.5f, 8f)); empty(\"Door_North\", new Vector3(0f, 1f, 12f)); empty(\"Door_South\", new Vector3(0f, 1f, -12f)); empty(\"Door_East\", new Vector3(12f, 1f, 0f)); empty(\"Door_West\", new Vector3(-12f, 1f, 0f)); empty(\"RunManager\", Vector3.zero); var player = GameObject.Find(\"Player\"); if (player == null) { player = GameObject.CreatePrimitive(PrimitiveType.Capsule); player.name = \"Player\"; UnityEditor.Undo.RegisterCreatedObjectUndo(player, \"Player\"); } player.transform.position = new Vector3(0f, 1.1f, 0f); var rb = player.GetComponent<Rigidbody>() ?? UnityEditor.Undo.AddComponent<Rigidbody>(player); rb.constraints = RigidbodyConstraints.FreezeRotation; var canvasGo = GameObject.Find(\"Canvas\"); if (canvasGo == null) { canvasGo = new GameObject(\"Canvas\"); var c = canvasGo.AddComponent<Canvas>(); c.renderMode = RenderMode.ScreenSpaceOverlay; canvasGo.AddComponent<UnityEngine.UI.CanvasScaler>(); canvasGo.AddComponent<UnityEngine.UI.GraphicRaycaster>(); UnityEditor.Undo.RegisterCreatedObjectUndo(canvasGo, \"Canvas\"); } var hud = GameObject.Find(\"RunHUD\"); if (hud == null) { hud = new GameObject(\"RunHUD\"); hud.transform.SetParent(canvasGo.transform, false); UnityEditor.Undo.RegisterCreatedObjectUndo(hud, \"RunHUD\"); System.Action<string,string,Vector2> label = (n, text, anchored) => { var go = new GameObject(n); go.transform.SetParent(hud.transform, false); var t = go.AddComponent<UnityEngine.UI.Text>(); t.font = Resources.GetBuiltinResource<Font>(\"Arial.ttf\"); t.text = text; t.fontSize = 22; t.color = Color.white; var rt = go.GetComponent<RectTransform>(); rt.anchorMin = new Vector2(0,1); rt.anchorMax = new Vector2(0,1); rt.pivot = new Vector2(0,1); rt.anchoredPosition = anchored; rt.sizeDelta = new Vector2(260, 36); }; label(\"RunHUD_Floor\", \"Floor 1\", new Vector2(16,-16)); label(\"RunHUD_HP\", \"HP 3\", new Vector2(16,-56)); } UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(UnityEngine.SceneManagement.SceneManager.GetActiveScene()); return \"roguelike layout: rooms Enemy_Spawn Door_North RunManager RunHUD\";";

fn eval_save_named_scene(path: &str) -> String {
    format!(
        "if (!UnityEditor.AssetDatabase.IsValidFolder(\"Assets/Scenes\")) UnityEditor.AssetDatabase.CreateFolder(\"Assets\", \"Scenes\"); var path = \"{path}\"; var scene = UnityEngine.SceneManagement.SceneManager.GetActiveScene(); UnityEditor.SceneManagement.EditorSceneManager.SaveScene(scene, path); var list = new System.Collections.Generic.List<UnityEditor.EditorBuildSettingsScene>(UnityEditor.EditorBuildSettings.scenes); bool found = false; foreach (var s in list) {{ if (s.path == path) {{ found = true; break; }} }} if (!found) list.Insert(0, new UnityEditor.EditorBuildSettingsScene(path, true)); UnityEditor.EditorBuildSettings.scenes = list.ToArray(); UnityEditor.AssetDatabase.Refresh(); return path;"
    )
}
const EVAL_LIST_SCENES: &str = "var scenes = UnityEditor.EditorBuildSettings.scenes; if (scenes == null || scenes.Length == 0) return \"(no build scenes)\"; var sb = new System.Text.StringBuilder(); for (int i = 0; i < scenes.Length; i++) { sb.Append(scenes[i].enabled ? \"[x] \" : \"[ ] \"); sb.Append(scenes[i].path); if (i + 1 < scenes.Length) sb.Append('\\n'); } return sb.ToString();";
const EVAL_NEW_SCENE: &str = "var scene = UnityEditor.SceneManagement.EditorSceneManager.NewScene(UnityEditor.SceneManagement.NewSceneSetup.DefaultGameObjects, UnityEditor.SceneManagement.NewSceneMode.Single); return scene.path.Length == 0 ? scene.name : scene.path;";
const EVAL_LOAD_FIRST_SCENE: &str = "var scenes = UnityEditor.EditorBuildSettings.scenes; if (scenes == null || scenes.Length == 0) return \"No scenes in Build Settings\"; var path = scenes[0].path; UnityEditor.SceneManagement.EditorSceneManager.OpenScene(path); return path;";
const EVAL_HIERARCHY_ROOTS: &str = "var roots = UnityEngine.SceneManagement.SceneManager.GetActiveScene().GetRootGameObjects(); if (roots == null || roots.Length == 0) return \"(empty)\"; var sb = new System.Text.StringBuilder(); for (int i = 0; i < roots.Length; i++) { sb.Append(roots[i].name); if (i + 1 < roots.Length) sb.Append('\\n'); } return sb.ToString();";
const EVAL_ACTIVE_SCENE: &str = "var scene = UnityEngine.SceneManagement.SceneManager.GetActiveScene(); return scene.path.Length == 0 ? scene.name : scene.path;";
const EVAL_SAVE_ASSETS: &str = "UnityEditor.AssetDatabase.SaveAssets(); return true;";
const EVAL_CONSOLE_ERRORS: &str = "var asm = System.Reflection.Assembly.GetAssembly(typeof(UnityEditor.Editor)); var t = asm.GetType(\"UnityEditor.LogEntries\"); var getCount = t.GetMethod(\"GetCount\"); int total = (int)getCount.Invoke(null, null); var getEntry = t.GetMethod(\"GetEntryInternal\"); var entryType = asm.GetType(\"UnityEditor.LogEntry\"); var entry = System.Activator.CreateInstance(entryType); var modeField = entryType.GetField(\"mode\"); var msgField = entryType.GetField(\"message\"); int errors = 0; var sb = new System.Text.StringBuilder(); for (int i = 0; i < total && errors < 20; i++) { getEntry.Invoke(null, new object[] { i, entry }); int mode = (int)modeField.GetValue(entry); if ((mode & 1) == 0) continue; errors++; sb.Append(msgField.GetValue(entry)); sb.Append('\\n'); } return \"errors=\" + errors + \"/\" + total + \"\\n\" + sb.ToString();";
const EVAL_MISSING_SCRIPTS: &str = "int missing = 0; var hits = new System.Collections.Generic.List<string>(); foreach (var go in UnityEngine.Object.FindObjectsByType<GameObject>(UnityEngine.FindObjectsSortMode.None)) { var comps = go.GetComponents<Component>(); for (int i = 0; i < comps.Length; i++) { if (comps[i] == null) { missing++; if (hits.Count < 20) hits.Add(go.name); break; } } } return \"missing=\" + missing + (hits.Count == 0 ? \"\" : \"\\n\" + string.Join(\"\\n\", hits));";
const EVAL_LIST_PACKAGES: &str = "var req = UnityEditor.PackageManager.Client.List(true); while (!req.IsCompleted) System.Threading.Thread.Sleep(50); if (req.Status != UnityEditor.PackageManager.StatusCode.Success) return req.Error != null ? req.Error.message : \"List failed\"; var sb = new System.Text.StringBuilder(); int n = 0; foreach (var p in req.Result) { if (n++ >= 40) break; sb.Append(p.name).Append('@').Append(p.version).Append('\\n'); } return \"count=\" + req.Result.Count() + \"\\n\" + sb.ToString();";
const EVAL_BUILD_WIN64: &str = "var enabled = System.Array.FindAll(UnityEditor.EditorBuildSettings.scenes, s => s.enabled); if (enabled.Length == 0) return \"No enabled scenes in Build Settings\"; var paths = System.Array.ConvertAll(enabled, s => s.path); var dir = System.IO.Path.GetFullPath(System.IO.Path.Combine(UnityEngine.Application.dataPath, \"..\", \"Builds\", \"Win64\")); System.IO.Directory.CreateDirectory(dir); var loc = System.IO.Path.Combine(dir, \"Player.exe\"); var report = UnityEditor.BuildPipeline.BuildPlayer(paths, loc, UnityEditor.BuildTarget.StandaloneWindows64, UnityEditor.BuildOptions.None); return report.summary.result.ToString() + \" \" + loc;";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliStatus {
    Unknown,
    Checking,
    Missing,
    Installing,
    Ready,
    Error,
}

impl CliStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown | Self::Checking => "检测中",
            Self::Installing => "安装中",
            Self::Missing => "未安装",
            Self::Ready => "已就绪",
            Self::Error => "异常",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopPhase {
    Observe,
    Act,
    Verify,
}

impl LoopPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Observe => "观察",
            Self::Act => "行动",
            Self::Verify => "验证",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Observe => "读取现场状态：场景、碰撞体、Play Mode",
            Self::Act => "通过 command / eval 热修复，无需域重载",
            Self::Verify => "重进 Play Mode，确认结果并回报 agent",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Observe => Self::Act,
            Self::Act => Self::Verify,
            Self::Verify => Self::Observe,
        }
    }
}

/// Live scene snapshot driven by observe / act / verify steps.
#[derive(Debug, Clone)]
pub struct SceneSnapshot {
    pub player_y: f32,
    pub ground_collider_enabled: bool,
    pub is_playing: bool,
    pub last_eval_result: String,
    pub note: String,
}

impl Default for SceneSnapshot {
    fn default() -> Self {
        Self {
            player_y: 1.0,
            ground_collider_enabled: true,
            is_playing: false,
            last_eval_result: "—".into(),
            note: "尚未观察场景".into(),
        }
    }
}

impl SceneSnapshot {
    pub fn status_line(&self) -> String {
        format!(
            "Player.y={:.1} · Collider={} · Play={}",
            self.player_y,
            if self.ground_collider_enabled {
                "ON"
            } else {
                "OFF"
            },
            if self.is_playing { "ON" } else { "OFF" }
        )
    }
}

#[derive(Debug, Clone)]
pub struct OpRecord {
    pub id: u64,
    pub title: String,
    pub command: String,
    pub phase: LoopPhase,
    pub ok: bool,
    pub summary: String,
    pub detail: String,
    pub at_unix: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStatus {
    Unknown,
    Checking,
    NotInstalled,
    Installing,
    PendingImport,
    Installed,
    Error,
}

impl PipelineStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "未知",
            Self::Checking => "检查中",
            Self::NotInstalled => "未安装",
            Self::Installing => "安装中",
            Self::PendingImport => "等待编辑器加载",
            Self::Installed => "已安装",
            Self::Error => "异常",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorLinkStatus {
    Unknown,
    Checking,
    Disconnected,
    Connected,
}

impl EditorLinkStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "未知",
            Self::Checking => "探测中",
            Self::Disconnected => "未连接",
            Self::Connected => "已连接",
        }
    }
}

/// Genre templates for one-click scene scaffolds (skeleton only, not full games).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameGenre {
    Playground,
    Rpg,
    Mmo,
    Roguelike,
}

impl GameGenre {
    pub fn label(self) -> &'static str {
        match self {
            Self::Playground => "小游戏",
            Self::Rpg => "RPG",
            Self::Mmo => "MMO",
            Self::Roguelike => "肉鸽",
        }
    }

    pub fn scene_path(self) -> &'static str {
        match self {
            Self::Playground => "Assets/Scenes/BonyPlayground.unity",
            Self::Rpg => "Assets/Scenes/BonyRpgTown.unity",
            Self::Mmo => "Assets/Scenes/BonyMmoHub.unity",
            Self::Roguelike => "Assets/Scenes/BonyRoguelikeRun.unity",
        }
    }

    pub fn done_toast(self) -> &'static str {
        match self {
            Self::Playground => "小游戏雏形搭建完成 ✓",
            Self::Rpg => "RPG 城镇骨架搭建完成 ✓",
            Self::Mmo => "MMO 大厅骨架搭建完成 ✓",
            Self::Roguelike => "肉鸽局骨架搭建完成 ✓",
        }
    }

    pub fn start_toast(self) -> &'static str {
        match self {
            Self::Playground => "开始搭建小游戏雏形",
            Self::Rpg => "开始搭建 RPG 城镇骨架",
            Self::Mmo => "开始搭建 MMO 大厅骨架",
            Self::Roguelike => "开始搭建肉鸽局骨架",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnityAction {
    RefreshDetect,
    ListEditors,
    ListPipeline,
    InstallPipeline,
    ListCommands,
    ProbeEditor,
    Eval,
    ObserveCollider,
    FixCollider,
    EnterPlayMode,
    ExitPlayMode,
    RunFullLoop,
    ScaffoldMiniGame,
    ScaffoldRpg,
    ScaffoldMmo,
    ScaffoldRoguelike,
    // Productivity: one-click editor ops (via Pipeline eval).
    SaveScene,
    RefreshAssets,
    RequestScriptReload,
    ClearConsole,
    PausePlayMode,
    StepPlayMode,
    UndoLast,
    RedoLast,
    FrameSelection,
    FocusGameView,
    FocusSceneView,
    DuplicateSelection,
    DeleteSelection,
    // A: scene / objects
    ListScenes,
    NewScene,
    LoadFirstScene,
    HierarchyRoots,
    ActiveSceneInfo,
    CreatePlane,
    CreateDirectionalLight,
    SelectLoopObject,
    // Game creation scaffold pieces.
    SetupSkyDay,
    SetupSkySunset,
    SetupSkyNight,
    CreateGround,
    SetupMainCamera,
    CreatePlayerCapsule,
    CreateNpc,
    CreateNpcVendor,
    CreateNpcQuest,
    CreateSpawnPoint,
    CreatePortalZone,
    CreateEnemySpawn,
    InstallNpcAi,
    AttachNpcAi,
    EnableNpcAi,
    LayoutRpg,
    LayoutMmo,
    LayoutRoguelike,
    SaveNamedScene,
    // B: assets / console
    SaveAssets,
    FindAssets,
    ConsoleErrors,
    FindMissingScripts,
    // C: packages
    ListPackages,
    AddPackage,
    // Productivity: project / connection CLI.
    EditorStatus,
    ListProjects,
    OpenProject,
    RequireEditor,
    UpgradePipeline,
    ProjectInfo,
    RegisterProject,
    PinProject,
    ListLtsReleases,
    HubLogs,
    CacheInfo,
    // Productivity: tests + D: build.
    RunEditModeTests,
    RunPlayModeTests,
    BuildWindowsPlayer,
    // Occasional diagnostics (slash only).
    Doctor,
    EnvInfo,
    LicenseInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GuideKind {
    #[default]
    None,
    Loop,
    Scaffold,
    NpcAi,
}

impl UnityAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::RefreshDetect => "重新检测",
            Self::ListEditors => "列出编辑器",
            Self::ListPipeline => "刷新 Pipeline",
            Self::InstallPipeline => "安装 Pipeline",
            Self::ListCommands => "发现命令",
            Self::ProbeEditor => "探测编辑器",
            Self::Eval => "运行 Eval",
            Self::ObserveCollider => "观察碰撞体",
            Self::FixCollider => "修复碰撞体",
            Self::EnterPlayMode => "进入 Play",
            Self::ExitPlayMode => "退出 Play",
            Self::RunFullLoop => "跑完整闭环",
            Self::ScaffoldMiniGame => "搭小游戏雏形",
            Self::ScaffoldRpg => "搭 RPG",
            Self::ScaffoldMmo => "搭 MMO 大厅",
            Self::ScaffoldRoguelike => "搭肉鸽局",
            Self::SaveScene => "保存场景",
            Self::RefreshAssets => "刷新资源",
            Self::RequestScriptReload => "重编译脚本",
            Self::ClearConsole => "清控制台",
            Self::PausePlayMode => "暂停/继续",
            Self::StepPlayMode => "单帧步进",
            Self::UndoLast => "撤销",
            Self::RedoLast => "重做",
            Self::FrameSelection => "框选聚焦",
            Self::FocusGameView => "切到 Game",
            Self::FocusSceneView => "切到 Scene",
            Self::DuplicateSelection => "复制选中",
            Self::DeleteSelection => "删除选中",
            Self::ListScenes => "列构建场景",
            Self::NewScene => "新建场景",
            Self::LoadFirstScene => "加载首场景",
            Self::HierarchyRoots => "场景根物体",
            Self::ActiveSceneInfo => "当前场景",
            Self::CreatePlane => "创建平面",
            Self::CreateDirectionalLight => "创建平行光",
            Self::SelectLoopObject => "选中闭环对象",
            Self::SetupSkyDay => "白天天空",
            Self::SetupSkySunset => "晚霞天空",
            Self::SetupSkyNight => "夜空",
            Self::CreateGround => "创建地面",
            Self::SetupMainCamera => "设置主相机",
            Self::CreatePlayerCapsule => "创建玩家",
            Self::CreateNpc => "创建 NPC",
            Self::CreateNpcVendor => "创建商人 NPC",
            Self::CreateNpcQuest => "创建任务 NPC",
            Self::CreateSpawnPoint => "创建出生点",
            Self::CreatePortalZone => "创建传送门",
            Self::CreateEnemySpawn => "创建敌人点",
            Self::InstallNpcAi => "安装 NPC AI 脚本",
            Self::AttachNpcAi => "挂载 NPC AI",
            Self::EnableNpcAi => "给 NPC 接入 AI",
            Self::LayoutRpg => "RPG 布局",
            Self::LayoutMmo => "MMO 布局",
            Self::LayoutRoguelike => "肉鸽布局",
            Self::SaveNamedScene => "保存雏形场景",
            Self::SaveAssets => "保存资源",
            Self::FindAssets => "搜索资源",
            Self::ConsoleErrors => "控制台错误",
            Self::FindMissingScripts => "缺失脚本",
            Self::ListPackages => "列出包",
            Self::AddPackage => "安装包",
            Self::EditorStatus => "编辑器状态",
            Self::ListProjects => "Hub 工程",
            Self::OpenProject => "打开工程",
            Self::RequireEditor => "补齐编辑器",
            Self::UpgradePipeline => "升级 Pipeline",
            Self::ProjectInfo => "工程信息",
            Self::RegisterProject => "注册到 Hub",
            Self::PinProject => "收藏工程",
            Self::ListLtsReleases => "LTS 版本",
            Self::HubLogs => "Hub 日志",
            Self::CacheInfo => "下载缓存",
            Self::RunEditModeTests => "EditMode 测试",
            Self::RunPlayModeTests => "PlayMode 测试",
            Self::BuildWindowsPlayer => "构建 Win64",
            Self::Doctor => "环境诊断",
            Self::EnvInfo => "Hub 环境",
            Self::LicenseInfo => "许可信息",
        }
    }

    /// Eval-style command results need JSON success parsing.
    fn is_eval_style(self) -> bool {
        matches!(
            self,
            Self::Eval
                | Self::ObserveCollider
                | Self::FixCollider
                | Self::EnterPlayMode
                | Self::ExitPlayMode
                | Self::SaveScene
                | Self::RefreshAssets
                | Self::RequestScriptReload
                | Self::ClearConsole
                | Self::PausePlayMode
                | Self::StepPlayMode
                | Self::UndoLast
                | Self::RedoLast
                | Self::FrameSelection
                | Self::FocusGameView
                | Self::FocusSceneView
                | Self::DuplicateSelection
                | Self::DeleteSelection
                | Self::ListScenes
                | Self::NewScene
                | Self::LoadFirstScene
                | Self::HierarchyRoots
                | Self::ActiveSceneInfo
                | Self::CreatePlane
                | Self::CreateDirectionalLight
                | Self::SelectLoopObject
                | Self::SetupSkyDay
                | Self::SetupSkySunset
                | Self::SetupSkyNight
                | Self::CreateGround
                | Self::SetupMainCamera
                | Self::CreatePlayerCapsule
                | Self::CreateNpc
                | Self::CreateNpcVendor
                | Self::CreateNpcQuest
                | Self::CreateSpawnPoint
                | Self::CreatePortalZone
                | Self::CreateEnemySpawn
                | Self::InstallNpcAi
                | Self::AttachNpcAi
                | Self::LayoutRpg
                | Self::LayoutMmo
                | Self::LayoutRoguelike
                | Self::SaveNamedScene
                | Self::SaveAssets
                | Self::FindAssets
                | Self::ConsoleErrors
                | Self::FindMissingScripts
                | Self::ListPackages
                | Self::AddPackage
                | Self::BuildWindowsPlayer
        )
    }

    fn timeout(self) -> Duration {
        match self {
            Self::InstallPipeline
            | Self::UpgradePipeline
            | Self::RequireEditor
            | Self::OpenProject => INSTALL_TIMEOUT,
            Self::RunEditModeTests | Self::RunPlayModeTests => TEST_TIMEOUT,
            Self::BuildWindowsPlayer => BUILD_TIMEOUT,
            Self::ListPackages | Self::AddPackage => PACKAGE_TIMEOUT,
            _ => CMD_TIMEOUT,
        }
    }
}

pub const EVAL_PRESETS: &[(&str, &str)] = &[
    ("Play?", "return UnityEditor.EditorApplication.isPlaying;"),
    ("Version", "return Application.version;"),
    ("DataPath", "return Application.dataPath;"),
    (
        "Collider",
        "var go = GameObject.Find(\"Ground\"); var c = go != null ? go.GetComponent<Collider>() : null; return c != null && c.enabled;",
    ),
];

const CREATE_SPHERE_EVAL: &str = "var go = GameObject.Find(\"BonySphere\"); if (go == null) { go = GameObject.CreatePrimitive(PrimitiveType.Sphere); go.name = \"BonySphere\"; } go.transform.position = Vector3.zero; UnityEditor.Selection.activeGameObject = go; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); return go.name;";

/// Chat → local Unity CLI (bypasses the coding agent).
#[derive(Debug, Clone, Copy)]
pub struct UnityChatCmd {
    pub chip: &'static str,
    pub slash: &'static str,
    pub action: UnityAction,
    pub eval: Option<&'static str>,
    /// Compact phrases (no spaces); matched with `contains` after normalize.
    pub phrases: &'static [&'static str],
}

/// Primary chips shown in the chat composer / empty state.
pub const UNITY_CHAT_CHIPS: &[UnityChatCmd] = &[
    UnityChatCmd {
        chip: "保存场景",
        slash: "/unity save",
        action: UnityAction::SaveScene,
        eval: None,
        phrases: &["保存场景", "存场景", "保存unity场景", "unitysave"],
    },
    UnityChatCmd {
        chip: "刷新资源",
        slash: "/unity refresh",
        action: UnityAction::RefreshAssets,
        eval: None,
        phrases: &["刷新资源", "刷资源", "assetrefresh", "unityrefresh"],
    },
    UnityChatCmd {
        chip: "清控制台",
        slash: "/unity clear",
        action: UnityAction::ClearConsole,
        eval: None,
        phrases: &["清控制台", "清空控制台", "清除控制台", "unityclear"],
    },
    UnityChatCmd {
        chip: "进入 Play",
        slash: "/unity play",
        action: UnityAction::EnterPlayMode,
        eval: None,
        phrases: &["进入play", "开始播放", "开始play", "unityplay"],
    },
    UnityChatCmd {
        chip: "退出 Play",
        slash: "/unity stop",
        action: UnityAction::ExitPlayMode,
        eval: None,
        phrases: &["退出play", "停止播放", "停止play", "unitystop"],
    },
    UnityChatCmd {
        chip: "搭 RPG",
        slash: "/unity scaffold rpg",
        action: UnityAction::ScaffoldRpg,
        eval: None,
        phrases: &[
            "搭rpg",
            "做一个rpg",
            "创建rpg",
            "rpg雏形",
            "rpg城镇",
            "搭rpg城镇",
            "scaffoldrpg",
            "unityscaffoldrpg",
        ],
    },
    UnityChatCmd {
        chip: "搭 MMO 大厅",
        slash: "/unity scaffold mmo",
        action: UnityAction::ScaffoldMmo,
        eval: None,
        phrases: &[
            "搭mmo大厅",
            "搭mmo",
            "做一个mmo",
            "创建mmo",
            "mmo大厅",
            "mmo雏形",
            "scaffoldmmo",
            "unityscaffoldmmo",
        ],
    },
    UnityChatCmd {
        chip: "搭肉鸽局",
        slash: "/unity scaffold roguelike",
        action: UnityAction::ScaffoldRoguelike,
        eval: None,
        phrases: &[
            "搭肉鸽局",
            "创建肉鸽关卡",
            "肉鸽雏形",
            "roguelike雏形",
            "搭roguelike",
            "做一个肉鸽",
            "scaffoldroguelike",
            "unityscaffoldroguelike",
        ],
    },
    UnityChatCmd {
        chip: "搭小游戏雏形",
        slash: "/unity scaffold",
        action: UnityAction::ScaffoldMiniGame,
        eval: None,
        phrases: &[
            "搭小游戏雏形",
            "做一个小游戏雏形",
            "搭一个基础场景",
            "一键搭场景",
            "搭建小游戏",
            "unityscaffold",
            "scaffoldminigame",
        ],
    },
    UnityChatCmd {
        chip: "创建 NPC",
        slash: "/unity npc",
        action: UnityAction::CreateNpc,
        eval: None,
        phrases: &[
            "创建npc",
            "生成npc",
            "新建npc",
            "放一个npc",
            "添加npc",
            "unitynpc",
            "createnpc",
        ],
    },
    UnityChatCmd {
        chip: "创建商人 NPC",
        slash: "/unity npc vendor",
        action: UnityAction::CreateNpcVendor,
        eval: None,
        phrases: &[
            "创建商人npc",
            "创建商人",
            "生成商人",
            "npcvendor",
            "创建npcvendor",
            "unitynpcvendor",
        ],
    },
    UnityChatCmd {
        chip: "创建任务 NPC",
        slash: "/unity npc quest",
        action: UnityAction::CreateNpcQuest,
        eval: None,
        phrases: &[
            "创建任务npc",
            "创建任务人",
            "生成任务npc",
            "npcquest",
            "创建npcquest",
            "unitynpcquest",
        ],
    },
    UnityChatCmd {
        chip: "给 NPC 接入 AI",
        slash: "/unity npc ai",
        action: UnityAction::EnableNpcAi,
        eval: None,
        phrases: &[
            "给npc接入ai",
            "npc接入ai",
            "接入npcai",
            "启用npcai",
            "安装npcai",
            "npcai",
            "unitynpcai",
            "enablenpcai",
        ],
    },
    UnityChatCmd {
        chip: "白天天空",
        slash: "/unity sky day",
        action: UnityAction::SetupSkyDay,
        eval: None,
        phrases: &["白天天空", "unityskyday"],
    },
    UnityChatCmd {
        chip: "晚霞天空",
        slash: "/unity sky sunset",
        action: UnityAction::SetupSkySunset,
        eval: None,
        phrases: &["晚霞天空", "unityskysunset"],
    },
    UnityChatCmd {
        chip: "暂停/继续",
        slash: "/unity pause",
        action: UnityAction::PausePlayMode,
        eval: None,
        phrases: &["暂停play", "暂停播放", "继续play", "unitypause"],
    },
    UnityChatCmd {
        chip: "单帧步进",
        slash: "/unity step",
        action: UnityAction::StepPlayMode,
        eval: None,
        phrases: &["单帧步进", "步进一帧", "unitystep"],
    },
    UnityChatCmd {
        chip: "撤销",
        slash: "/unity undo",
        action: UnityAction::UndoLast,
        eval: None,
        phrases: &["撤销", "unity撤销", "unityundo"],
    },
    UnityChatCmd {
        chip: "框选聚焦",
        slash: "/unity frame",
        action: UnityAction::FrameSelection,
        eval: None,
        phrases: &["框选聚焦", "聚焦选中", "unityframe", "框住选中"],
    },
    UnityChatCmd {
        chip: "切到 Game",
        slash: "/unity game",
        action: UnityAction::FocusGameView,
        eval: None,
        phrases: &["切到game", "打开game视图", "unitygame"],
    },
    UnityChatCmd {
        chip: "创建球体",
        slash: "/unity sphere",
        action: UnityAction::Eval,
        eval: Some(CREATE_SPHERE_EVAL),
        phrases: &[
            "创建球体",
            "新建球体",
            "生成球体",
            "画一个球体",
            "画个球体",
            "场景画一个球体",
            "场景里放一个球体",
            "unitysphere",
        ],
    },
    UnityChatCmd {
        chip: "创建立方体",
        slash: "/unity cube",
        action: UnityAction::Eval,
        eval: Some(EVAL_CREATE_CUBE),
        phrases: &["创建立方体", "新建立方体", "生成立方体", "unitycube"],
    },
    UnityChatCmd {
        chip: "创建平面",
        slash: "/unity plane",
        action: UnityAction::CreatePlane,
        eval: None,
        phrases: &["创建平面", "新建平面", "生成平面", "unityplane"],
    },
    UnityChatCmd {
        chip: "列场景",
        slash: "/unity scenes",
        action: UnityAction::ListScenes,
        eval: None,
        phrases: &["列场景", "列出场景", "构建场景", "unityscenes"],
    },
    UnityChatCmd {
        chip: "控制台错误",
        slash: "/unity errors",
        action: UnityAction::ConsoleErrors,
        eval: None,
        phrases: &["控制台错误", "查看错误", "unityerrors", "console错误"],
    },
    UnityChatCmd {
        chip: "构建 Win64",
        slash: "/unity build win",
        action: UnityAction::BuildWindowsPlayer,
        eval: None,
        phrases: &["构建win64", "打包windows", "unitybuild", "unitybuildwin"],
    },
    UnityChatCmd {
        chip: "注册到 Hub",
        slash: "/unity register",
        action: UnityAction::RegisterProject,
        eval: None,
        phrases: &["注册到hub", "注册工程", "添加到hub", "unityregister"],
    },
    UnityChatCmd {
        chip: "编辑器状态",
        slash: "/unity status",
        action: UnityAction::EditorStatus,
        eval: None,
        phrases: &["编辑器状态", "unity状态", "unitystatus"],
    },
    UnityChatCmd {
        chip: "打开工程",
        slash: "/unity open",
        action: UnityAction::OpenProject,
        eval: None,
        phrases: &["打开工程", "打开unity工程", "打开项目", "unityopen"],
    },
    UnityChatCmd {
        chip: "EditMode 测试",
        slash: "/unity test edit",
        action: UnityAction::RunEditModeTests,
        eval: None,
        phrases: &["editmode测试", "跑editmode", "unitytested", "unitytestedit"],
    },
    UnityChatCmd {
        chip: "探测编辑器",
        slash: "/unity probe",
        action: UnityAction::ProbeEditor,
        eval: None,
        phrases: &["探测编辑器", "检查编辑器", "连接编辑器", "unityprobe"],
    },
];

/// Extra slash / phrase matches not shown as chips.
pub const UNITY_CHAT_EXTRA: &[UnityChatCmd] = &[
    UnityChatCmd {
        chip: "夜空",
        slash: "/unity sky night",
        action: UnityAction::SetupSkyNight,
        eval: None,
        phrases: &["夜空", "unityskynight"],
    },
    UnityChatCmd {
        chip: "创建地面",
        slash: "/unity ground",
        action: UnityAction::CreateGround,
        eval: None,
        phrases: &["创建地面", "铺地面", "unityground"],
    },
    UnityChatCmd {
        chip: "设置主相机",
        slash: "/unity camera",
        action: UnityAction::SetupMainCamera,
        eval: None,
        phrases: &["设置主相机", "摆相机", "unitycamera"],
    },
    UnityChatCmd {
        chip: "创建玩家",
        slash: "/unity player",
        action: UnityAction::CreatePlayerCapsule,
        eval: None,
        phrases: &["创建玩家", "生成玩家", "unityplayer"],
    },
    UnityChatCmd {
        chip: "创建出生点",
        slash: "/unity spawn",
        action: UnityAction::CreateSpawnPoint,
        eval: None,
        phrases: &[
            "创建出生点",
            "创建spawn点",
            "生成出生点",
            "添加spawn",
            "unityspawn",
            "createspawn",
        ],
    },
    UnityChatCmd {
        chip: "创建传送门",
        slash: "/unity portal",
        action: UnityAction::CreatePortalZone,
        eval: None,
        phrases: &[
            "创建传送门",
            "创建portal",
            "生成传送门",
            "portalzone",
            "unityportal",
        ],
    },
    UnityChatCmd {
        chip: "创建敌人点",
        slash: "/unity enemy spawn",
        action: UnityAction::CreateEnemySpawn,
        eval: None,
        phrases: &[
            "创建敌人点",
            "创建敌人出生点",
            "生成敌人点",
            "enemyspawn",
            "unityenemyspawn",
        ],
    },
    UnityChatCmd {
        chip: "安装 NPC AI 脚本",
        slash: "/unity npc ai install",
        action: UnityAction::InstallNpcAi,
        eval: None,
        phrases: &["安装npcai脚本", "安装npcai", "unitynpcaiinstall"],
    },
    UnityChatCmd {
        chip: "挂载 NPC AI",
        slash: "/unity npc ai attach",
        action: UnityAction::AttachNpcAi,
        eval: None,
        phrases: &["挂载npcai", "附加npcai", "unitynpcaiattach"],
    },
    UnityChatCmd {
        chip: "RPG 布局",
        slash: "/unity layout rpg",
        action: UnityAction::LayoutRpg,
        eval: None,
        phrases: &["rpg布局", "unitylayoutrpg"],
    },
    UnityChatCmd {
        chip: "MMO 布局",
        slash: "/unity layout mmo",
        action: UnityAction::LayoutMmo,
        eval: None,
        phrases: &["mmo布局", "unitylayoutmmo"],
    },
    UnityChatCmd {
        chip: "肉鸽布局",
        slash: "/unity layout roguelike",
        action: UnityAction::LayoutRoguelike,
        eval: None,
        phrases: &["肉鸽布局", "roguelike布局", "unitylayoutroguelike"],
    },
    UnityChatCmd {
        chip: "保存雏形场景",
        slash: "/unity save playground",
        action: UnityAction::SaveNamedScene,
        eval: None,
        phrases: &["保存雏形场景", "保存playground", "unitysaveplayground"],
    },
    UnityChatCmd {
        chip: "重做",
        slash: "/unity redo",
        action: UnityAction::RedoLast,
        eval: None,
        phrases: &["重做", "unity重做", "unityredo"],
    },
    UnityChatCmd {
        chip: "切到 Scene",
        slash: "/unity scene",
        action: UnityAction::FocusSceneView,
        eval: None,
        phrases: &["切到scene", "打开scene视图", "unityscene"],
    },
    UnityChatCmd {
        chip: "复制选中",
        slash: "/unity duplicate",
        action: UnityAction::DuplicateSelection,
        eval: None,
        phrases: &["复制选中", "复制物体", "unityduplicate", "duplicate选中"],
    },
    UnityChatCmd {
        chip: "删除选中",
        slash: "/unity delete",
        action: UnityAction::DeleteSelection,
        eval: None,
        phrases: &["删除选中", "删掉选中", "unitydelete"],
    },
    UnityChatCmd {
        chip: "创建胶囊体",
        slash: "/unity capsule",
        action: UnityAction::Eval,
        eval: Some(EVAL_CREATE_CAPSULE),
        phrases: &["创建胶囊体", "新建胶囊体", "unitycapsule"],
    },
    UnityChatCmd {
        chip: "创建平行光",
        slash: "/unity light",
        action: UnityAction::CreateDirectionalLight,
        eval: None,
        phrases: &["创建平行光", "创建灯光", "新建灯光", "unitylight"],
    },
    UnityChatCmd {
        chip: "新建场景",
        slash: "/unity newscene",
        action: UnityAction::NewScene,
        eval: None,
        phrases: &["新建场景", "创建空场景", "unitynewscene"],
    },
    UnityChatCmd {
        chip: "加载首场景",
        slash: "/unity loadscene",
        action: UnityAction::LoadFirstScene,
        eval: None,
        phrases: &["加载首场景", "打开首场景", "unityloadscene"],
    },
    UnityChatCmd {
        chip: "场景根物体",
        slash: "/unity hierarchy",
        action: UnityAction::HierarchyRoots,
        eval: None,
        phrases: &["场景根物体", "列根物体", "unityhierarchy"],
    },
    UnityChatCmd {
        chip: "当前场景",
        slash: "/unity activescene",
        action: UnityAction::ActiveSceneInfo,
        eval: None,
        phrases: &["当前场景", "活动场景", "unityactivescene"],
    },
    UnityChatCmd {
        chip: "保存资源",
        slash: "/unity saveassets",
        action: UnityAction::SaveAssets,
        eval: None,
        phrases: &["保存资源", "存资源", "unitysaveassets"],
    },
    UnityChatCmd {
        chip: "搜索资源",
        slash: "/unity find",
        action: UnityAction::FindAssets,
        eval: None,
        phrases: &["搜索资源", "查找预制体", "unityfind", "findassets"],
    },
    UnityChatCmd {
        chip: "缺失脚本",
        slash: "/unity missing",
        action: UnityAction::FindMissingScripts,
        eval: None,
        phrases: &["缺失脚本", "丢失脚本", "unitymissing"],
    },
    UnityChatCmd {
        chip: "列出包",
        slash: "/unity packages",
        action: UnityAction::ListPackages,
        eval: None,
        phrases: &["列出包", "包列表", "unitypackages"],
    },
    UnityChatCmd {
        chip: "安装包",
        slash: "/unity addpackage",
        action: UnityAction::AddPackage,
        eval: None,
        phrases: &["安装包", "添加包", "unityaddpackage"],
    },
    UnityChatCmd {
        chip: "选中闭环对象",
        slash: "/unity selectloop",
        action: UnityAction::SelectLoopObject,
        eval: None,
        phrases: &["选中闭环对象", "选中闭环", "unityselectloop"],
    },
    UnityChatCmd {
        chip: "升级 Pipeline",
        slash: "/unity pipeline upgrade",
        action: UnityAction::UpgradePipeline,
        eval: None,
        phrases: &["升级pipeline", "pipeline升级", "unitypipelineupgrade"],
    },
    UnityChatCmd {
        chip: "工程信息",
        slash: "/unity info",
        action: UnityAction::ProjectInfo,
        eval: None,
        phrases: &["工程信息", "项目信息", "unityinfo", "unityprojectinfo"],
    },
    UnityChatCmd {
        chip: "收藏工程",
        slash: "/unity pin",
        action: UnityAction::PinProject,
        eval: None,
        phrases: &["收藏工程", "固定工程", "unitypin"],
    },
    UnityChatCmd {
        chip: "LTS 版本",
        slash: "/unity releases",
        action: UnityAction::ListLtsReleases,
        eval: None,
        phrases: &["lts版本", "可用lts", "unityreleases", "unitylts"],
    },
    UnityChatCmd {
        chip: "Hub 日志",
        slash: "/unity logs",
        action: UnityAction::HubLogs,
        eval: None,
        phrases: &["hub日志", "unity日志", "unitylogs"],
    },
    UnityChatCmd {
        chip: "下载缓存",
        slash: "/unity cache",
        action: UnityAction::CacheInfo,
        eval: None,
        phrases: &["下载缓存", "unity缓存", "unitycache"],
    },
    UnityChatCmd {
        chip: "重编译脚本",
        slash: "/unity recompile",
        action: UnityAction::RequestScriptReload,
        eval: None,
        phrases: &["重编译脚本", "强制重编译", "unityrecompile", "脚本重载"],
    },
    UnityChatCmd {
        chip: "Hub 工程",
        slash: "/unity projects",
        action: UnityAction::ListProjects,
        eval: None,
        phrases: &["hub工程", "列出工程", "unityprojects", "工程列表"],
    },
    UnityChatCmd {
        chip: "补齐编辑器",
        slash: "/unity require",
        action: UnityAction::RequireEditor,
        eval: None,
        phrases: &["补齐编辑器", "安装所需编辑器", "unityrequire"],
    },
    UnityChatCmd {
        chip: "PlayMode 测试",
        slash: "/unity test play",
        action: UnityAction::RunPlayModeTests,
        eval: None,
        phrases: &["playmode测试", "跑playmode", "unitytestplay"],
    },
    UnityChatCmd {
        chip: "跑闭环",
        slash: "/unity loop",
        action: UnityAction::RunFullLoop,
        eval: None,
        phrases: &["运行完整闭环", "跑完整闭环", "完整闭环", "unityloop"],
    },
    UnityChatCmd {
        chip: "查版本",
        slash: "/unity version",
        action: UnityAction::Eval,
        eval: Some(EVAL_PRESETS[1].1),
        phrases: &["查询unity版本", "查unity版本", "unity版本", "unityversion"],
    },
    UnityChatCmd {
        chip: "安装 Pipeline",
        slash: "/unity install",
        action: UnityAction::InstallPipeline,
        eval: None,
        phrases: &[
            "安装pipeline",
            "装pipeline",
            "unityinstall",
            "unitypipelineinstall",
        ],
    },
    UnityChatCmd {
        chip: "检测 CLI",
        slash: "/unity detect",
        action: UnityAction::RefreshDetect,
        eval: None,
        phrases: &["检测unity", "重新检测unity", "检测cli", "unitydetect"],
    },
    UnityChatCmd {
        chip: "刷新 Pipeline",
        slash: "/unity pipeline",
        action: UnityAction::ListPipeline,
        eval: None,
        phrases: &["刷新pipeline", "检查pipeline", "unitypipeline"],
    },
    UnityChatCmd {
        chip: "发现命令",
        slash: "/unity commands",
        action: UnityAction::ListCommands,
        eval: None,
        phrases: &["发现命令", "列出unity命令", "unitycommands"],
    },
    UnityChatCmd {
        chip: "查项目路径",
        slash: "/unity path",
        action: UnityAction::Eval,
        eval: Some(EVAL_PRESETS[2].1),
        phrases: &["查询项目路径", "查项目路径", "unitypath"],
    },
    UnityChatCmd {
        chip: "查碰撞体",
        slash: "/unity collider",
        action: UnityAction::Eval,
        eval: Some(EVAL_PRESETS[3].1),
        phrases: &["查询碰撞体", "查碰撞体", "unitycollider"],
    },
    UnityChatCmd {
        chip: "查播放状态",
        slash: "/unity playing",
        action: UnityAction::Eval,
        eval: Some(EVAL_PRESETS[0].1),
        phrases: &["查询播放状态", "查播放状态", "是否在play", "unityplaying"],
    },
    UnityChatCmd {
        chip: "环境诊断",
        slash: "/unity doctor",
        action: UnityAction::Doctor,
        eval: None,
        phrases: &["环境诊断", "unitydoctor", "unity诊断"],
    },
    UnityChatCmd {
        chip: "Hub 环境",
        slash: "/unity env",
        action: UnityAction::EnvInfo,
        eval: None,
        phrases: &["hub环境", "unityenv", "unity环境路径"],
    },
    UnityChatCmd {
        chip: "许可信息",
        slash: "/unity license",
        action: UnityAction::LicenseInfo,
        eval: None,
        phrases: &["许可信息", "unitylicense", "unity许可"],
    },
];

pub fn normalize_unity_chat(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn parse_unity_chat_command(text: &str) -> Option<&'static UnityChatCmd> {
    let n = normalize_unity_chat(text);
    if n.is_empty() {
        return None;
    }
    for cmd in UNITY_CHAT_CHIPS.iter().chain(UNITY_CHAT_EXTRA.iter()) {
        let slash_key = normalize_unity_chat(cmd.slash);
        if n == slash_key {
            return Some(cmd);
        }
        for p in cmd.phrases {
            if n == *p || n.contains(p) {
                return Some(cmd);
            }
        }
    }
    None
}

/// Compiles parameterized natural-language creation requests into a bounded
/// Unity Eval operation. This is intentionally data-driven (count + primitive)
/// rather than a growing list of exact phrases.
pub fn compile_unity_scene_command(text: &str) -> Option<(String, String)> {
    let n = normalize_unity_chat(text);
    // Sky presets (freer NL than exact chips).
    if n.contains("晚霞") || n.contains("日落") || n.contains("夕阳") {
        return Some(("晚霞天空".into(), EVAL_SETUP_SKY_SUNSET.into()));
    }
    if n.contains("夜空") || n.contains("夜晚天空") || (n.contains("黑夜") && n.contains("天")) {
        return Some(("夜空".into(), EVAL_SETUP_SKY_NIGHT.into()));
    }
    if n.contains("白天天空")
        || n.contains("蓝天")
        || n.contains("晴空")
        || n == "创建天空"
        || n.contains("换个天空")
        || n.contains("设置天空")
    {
        return Some(("白天天空".into(), EVAL_SETUP_SKY_DAY.into()));
    }
    if n.contains("删除选中") || n.contains("删除这些") || n.contains("移除选中") {
        return Some((
            "删除选中对象".into(),
            "var targets = UnityEditor.Selection.gameObjects; if (targets == null || targets.Length == 0) return \"No selected objects\"; int count = targets.Length; foreach (var target in targets) UnityEditor.Undo.DestroyObjectImmediate(target); return \"Deleted \" + count + \" objects\";".into(),
        ));
    }
    if n.contains("复制选中") || n.contains("复制这些") || n.contains("再复制") {
        return Some((
            "复制选中对象".into(),
            "var targets = UnityEditor.Selection.gameObjects; if (targets == null || targets.Length == 0) return \"No selected objects\"; var created = new System.Collections.Generic.List<GameObject>(); foreach (var target in targets) { var copy = UnityEngine.Object.Instantiate(target); copy.name = target.name + \" Copy\"; copy.transform.position += Vector3.right * 2f; UnityEditor.Undo.RegisterCreatedObjectUndo(copy, \"Duplicate object\"); created.Add(copy); UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(copy.scene); } UnityEditor.Selection.objects = created.ToArray(); return \"Duplicated \" + created.Count + \" objects\";".into(),
        ));
    }
    if n.contains("放大") || n.contains("缩小") {
        let factor = if n.contains("缩小") { "0.5f" } else { "2f" };
        return Some((
            if n.contains("缩小") {
                "缩小选中对象"
            } else {
                "放大选中对象"
            }
            .into(),
            format!(
                "var targets = UnityEditor.Selection.gameObjects; if (targets == null || targets.Length == 0) return \"No selected objects\"; foreach (var target in targets) {{ UnityEditor.Undo.RecordObject(target.transform, \"Scale object\"); target.transform.localScale *= {factor}; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(target.scene); }} return \"Scaled \" + targets.Length + \" objects\";"
            ),
        ));
    }
    let movement = if n.contains("向上") || n.contains("往上") {
        Some(("向上移动选中对象", "Vector3.up"))
    } else if n.contains("向下") || n.contains("往下") {
        Some(("向下移动选中对象", "Vector3.down"))
    } else if n.contains("向左") || n.contains("往左") {
        Some(("向左移动选中对象", "Vector3.left"))
    } else if n.contains("向右") || n.contains("往右") {
        Some(("向右移动选中对象", "Vector3.right"))
    } else if n.contains("向前") || n.contains("往前") {
        Some(("向前移动选中对象", "Vector3.forward"))
    } else if n.contains("向后") || n.contains("往后") {
        Some(("向后移动选中对象", "Vector3.back"))
    } else {
        None
    };
    if let Some((label, direction)) = movement {
        return Some((
            label.into(),
            format!(
                "var targets = UnityEditor.Selection.gameObjects; if (targets == null || targets.Length == 0) return \"No selected objects\"; foreach (var target in targets) {{ UnityEditor.Undo.RecordObject(target.transform, \"Move object\"); target.transform.position += {direction}; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(target.scene); }} return \"Moved \" + targets.Length + \" objects\";"
            ),
        ));
    }
    if n.contains("刚体") || n.contains("rigidbody") {
        return Some((
            "给选中对象添加刚体".into(),
            "var targets = UnityEditor.Selection.gameObjects; if (targets == null || targets.Length == 0) return \"No selected objects\"; int changed = 0; foreach (var target in targets) { foreach (var t in target.GetComponentsInChildren<Transform>(true)) { if (t.GetComponent<Rigidbody>() == null) { UnityEditor.Undo.AddComponent<Rigidbody>(t.gameObject); changed++; } } UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(target.scene); } return \"Added Rigidbody to \" + changed + \" objects\";".into(),
        ));
    }
    let color = [
        ("绿色", "green", "Color.green"),
        ("红色", "red", "Color.red"),
        ("蓝色", "blue", "Color.blue"),
        ("黄色", "yellow", "Color.yellow"),
        ("白色", "white", "Color.white"),
        ("黑色", "black", "Color.black"),
        ("灰色", "gray", "Color.gray"),
        ("青色", "cyan", "Color.cyan"),
        ("紫色", "magenta", "Color.magenta"),
    ]
    .iter()
    .find(|(cn, en, _)| n.contains(cn) || n.contains(en));
    if let Some((cn, _, unity_color)) = color {
        let eval = format!(
            "var targets = UnityEditor.Selection.gameObjects; if (targets == null || targets.Length == 0) return \"No selected objects\"; int changed = 0; var shader = Shader.Find(\"Universal Render Pipeline/Lit\") ?? Shader.Find(\"Standard\"); foreach (var target in targets) {{ foreach (var renderer in target.GetComponentsInChildren<Renderer>(true)) {{ UnityEditor.Undo.RecordObject(renderer, \"Set color\"); var material = new Material(shader); material.name = \"Bony {cn}\"; material.color = {unity_color}; renderer.sharedMaterial = material; changed++; }} UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(target.scene); }} return \"Colored \" + changed + \" renderers\";"
        );
        return Some((format!("把选中对象设为{cn}"), eval));
    }
    if !["创建", "新建", "生成", "放", "画"]
        .iter()
        .any(|verb| n.contains(verb))
    {
        return None;
    }
    if n.contains("npc") {
        let ascii_count = n
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
            .ok();
        let cn_count = [
            ("十", 10),
            ("九", 9),
            ("八", 8),
            ("七", 7),
            ("六", 6),
            ("五", 5),
            ("四", 4),
            ("三", 3),
            ("二", 2),
            ("两", 2),
            ("一", 1),
        ]
        .iter()
        .find_map(|(token, value)| n.contains(token).then_some(*value));
        let count = ascii_count.or(cn_count).unwrap_or(1).clamp(1, 50);
        let eval = format!(
            "var created = new System.Collections.Generic.List<GameObject>(); int next = 1; for (int n = 0; n < {count}; n++) {{ string name; do {{ name = \"NPC_\" + next; next++; }} while (GameObject.Find(name) != null); var go = GameObject.CreatePrimitive(PrimitiveType.Capsule); go.name = name; go.transform.position = new Vector3((next - 2) * 2f, 0.5f, 2f); UnityEditor.Undo.RegisterCreatedObjectUndo(go, name); created.Add(go); UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(go.scene); }} UnityEditor.Selection.objects = created.ToArray(); return \"Created \" + created.Count + \" NPCs\";"
        );
        return Some((format!("创建 {count} 个 NPC"), eval));
    }
    let (cn_name, primitive) = if n.contains("球体") || n.contains("sphere") {
        ("球体", "Sphere")
    } else if n.contains("立方体")
        || n.contains("正方体")
        || n.contains("方块")
        || n.contains("cube")
    {
        ("立方体", "Cube")
    } else if n.contains("胶囊") || n.contains("capsule") {
        ("胶囊体", "Capsule")
    } else if n.contains("圆柱") || n.contains("cylinder") {
        ("圆柱体", "Cylinder")
    } else if n.contains("平面") || n.contains("plane") {
        ("平面", "Plane")
    } else {
        return None;
    };
    let ascii_count = n
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<usize>()
        .ok();
    let cn_count = [
        ("十", 10),
        ("九", 9),
        ("八", 8),
        ("七", 7),
        ("六", 6),
        ("五", 5),
        ("四", 4),
        ("三", 3),
        ("二", 2),
        ("两", 2),
        ("一", 1),
    ]
    .iter()
    .find_map(|(token, value)| n.contains(token).then_some(*value));
    let count = ascii_count.or(cn_count).unwrap_or(1).clamp(1, 50);
    let prefix = format!("Bony{primitive}");
    let eval = format!(
        "var root = new GameObject(\"{prefix}Group\"); UnityEditor.Undo.RegisterCreatedObjectUndo(root, \"Create {count} {primitive}\"); int columns = Mathf.CeilToInt(Mathf.Sqrt({count})); for (int i = 0; i < {count}; i++) {{ var go = GameObject.CreatePrimitive(PrimitiveType.{primitive}); go.name = \"{prefix}_\" + (i + 1); go.transform.SetParent(root.transform); float x = (i % columns) * 2f - (columns - 1) * 1f; float z = (i / columns) * 2f; go.transform.position = new Vector3(x, 0f, z); }} UnityEditor.Selection.activeGameObject = root; UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(root.scene); if (UnityEditor.SceneView.lastActiveSceneView != null) UnityEditor.SceneView.lastActiveSceneView.FrameSelected(); return \"Created {count} {primitive}\";"
    );
    Some((format!("创建 {count} 个{cn_name}"), eval))
}

#[derive(serde::Deserialize)]
struct GeneratedUnityPlan {
    summary: String,
    csharp: String,
}

/// Parse the agent's generic Unity plan and reject APIs that can escape the
/// editor/project boundary. UnityEditor/UnityEngine remain available, allowing
/// arbitrary scene, asset, prefab, animation, UI and component operations.
pub fn parse_generated_unity_plan(raw: &str) -> Result<(String, String), String> {
    let (summary, csharp, risks) = parse_generated_unity_plan_unrestricted(raw)?;
    if let Some(api) = risks.first() {
        return Err(format!("Unity 计划包含禁止的越界 API：{api}"));
    }
    Ok((summary, csharp))
}

pub fn parse_generated_unity_plan_unrestricted(
    raw: &str,
) -> Result<(String, String, Vec<String>), String> {
    let trimmed = raw.trim();
    let json = if trimmed.starts_with("```") {
        let body = trimmed
            .split_once('\n')
            .map(|(_, body)| body)
            .unwrap_or(trimmed);
        body.rsplit_once("```")
            .map(|(body, _)| body)
            .unwrap_or(body)
    } else {
        trimmed
    };
    let plan: GeneratedUnityPlan = serde_json::from_str(json.trim())
        .map_err(|error| format!("Unity 计划格式无效：{error}"))?;
    if plan.summary.trim().is_empty() || plan.csharp.trim().is_empty() {
        return Err("Unity 计划缺少 summary 或 csharp".into());
    }
    if plan.csharp.len() > 24_000 {
        return Err("Unity 计划过长，已拒绝执行".into());
    }
    let lower = plan.csharp.to_ascii_lowercase();
    let blocked = [
        "system.io",
        "system.net",
        "system.diagnostics.process",
        "microsoft.win32",
        "dllimport",
        "marshal.",
        "environment.exit",
        "file.",
        "directory.",
        "webrequest",
        "httpclient",
        "reflection",
        "assembly.load",
    ];
    let risks = blocked
        .iter()
        .filter(|api| lower.contains(**api))
        .map(|api| (*api).to_string())
        .collect();
    Ok((
        plan.summary.trim().to_string(),
        plan.csharp.trim().to_string(),
        risks,
    ))
}

pub fn unity_chat_help_text() -> String {
    let mut lines = vec![
        "### 对话控制 Unity（本地 CLI，不经 Agent）".to_string(),
        String::new(),
        "在聊天输入框点 **Unity** 打开快捷指令，或直接发送：".to_string(),
        String::new(),
    ];
    lines.push(
        "提效常用：`搭小游戏雏形`、`白天天空`、`晚霞天空`、`保存场景`、`进入 Play`、`打开工程`。"
            .into(),
    );
    lines.push(String::new());
    lines.push(
        "创作流水线：`/unity scaffold` 会按序新建场景 → 天空 → 地面 → 灯光 → 相机 → 玩家 → 存到 Assets/Scenes/BonyPlayground.unity → 进 Play。"
            .into(),
    );
    lines.push(String::new());
    lines.push("自然语言场景操作支持数量和基础类型，例如：`创建3个球体`、`生成五个立方体`、`放两个胶囊体`。".into());
    lines.push(String::new());
    lines.push("闭环对象可在侧栏修改（默认 `Ground`）；搜索资源/安装包可先在 eval 框填过滤器或包名。".into());
    lines.push(String::new());
    lines.push("快捷命令：".into());
    lines.push(String::new());
    for cmd in UNITY_CHAT_CHIPS.iter().chain(UNITY_CHAT_EXTRA.iter()) {
        lines.push(format!("- **{}** · `{}`", cmd.chip, cmd.slash));
    }
    lines.push(String::new());
    lines.push(
        "首次使用请先在侧栏「插件 → Unity 控制 → 打开设置」选好工程根并完成引导（CLI → Pipeline → 探测）。".into(),
    );
    lines.join("\n")
}

pub fn wants_unity_help(text: &str) -> bool {
    let n = normalize_unity_chat(text);
    matches!(
        n.as_str(),
        "/unity"
            | "/unityhelp"
            | "/unity帮助"
            | "unity帮助"
            | "unity指令"
            | "unity命令"
            | "帮助unity"
    ) || n == "unity?"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    InstallCli,
    DetectCli,
    PickProject,
    InstallPipeline,
    ProbeEditor,
    RunLoop,
}

impl SetupStep {
    pub const ALL: [SetupStep; 6] = [
        Self::InstallCli,
        Self::DetectCli,
        Self::PickProject,
        Self::InstallPipeline,
        Self::ProbeEditor,
        Self::RunLoop,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::InstallCli => "安装 Unity CLI",
            Self::DetectCli => "检测 CLI",
            Self::PickProject => "确认 Unity 工程根（不要用 agent worktree）",
            Self::InstallPipeline => "安装 Pipeline",
            Self::ProbeEditor => "探测编辑器",
            Self::RunLoop => "跑闭环验证",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::InstallCli => "本机需要独立的 unity 命令行（不是编辑器 Unity.exe）",
            Self::DetectCli => "确认 PATH / UNITY_CLI 能找到 unity 二进制",
            Self::PickProject => "必须是含 Assets + ProjectSettings 的工程根；聊天任务目录无效",
            Self::InstallPipeline => "在项目中执行 unity pipeline install",
            Self::ProbeEditor => "编辑器打开同一工程后，unity command 才能响应",
            Self::RunLoop => "观察 → 行动 → 验证（演示或实机）",
        }
    }

    pub fn primary_label(self) -> &'static str {
        match self {
            Self::InstallCli => "复制安装命令",
            Self::DetectCli => "重新检测",
            Self::PickProject => "选择 Unity 工程…",
            Self::InstallPipeline => "安装 Pipeline",
            Self::ProbeEditor => "探测编辑器",
            Self::RunLoop => "跑完整闭环",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Locked,
}

#[derive(Debug)]
pub struct UnityState {
    pub status: CliStatus,
    pub cli_path: Option<PathBuf>,
    pub version_line: String,
    pub last_error: Option<String>,
    pub project_path: PathBuf,
    pub editors_json: String,
    pub editors_summary: String,
    pub pipeline_status: PipelineStatus,
    pub pipeline_summary: String,
    pub pipeline_detail: String,
    pub editor_link: EditorLinkStatus,
    pub commands_summary: String,
    pub eval_input: String,
    /// Object name used by observe/fix/select loop actions (default Ground).
    pub loop_object: String,
    pub busy: bool,
    pub loop_phase: LoopPhase,
    pub demo_mode: bool,
    pub scene: SceneSnapshot,
    pub guide_label: Option<String>,
    pub toast: Option<String>,
    pub history: Vec<OpRecord>,
    pub next_id: u64,
    /// Active onboarding step highlighted in the wizard.
    pub setup_step: SetupStep,
    /// User can expand earlier/later steps manually.
    pub setup_focus: Option<SetupStep>,
    /// When true, agent cwd must not overwrite the chosen Unity project path.
    pub project_locked: bool,
    /// Last cwd passed to `consider_agent_cwd` — skip disk probes when unchanged.
    last_considered_cwd: Option<PathBuf>,
    pending_rx: Option<mpsc::Receiver<(u64, UnityWorkerMsg)>>,
    /// Monotonically increasing id of the most recently spawned worker job;
    /// messages tagged with an older id are stale (superseded) and dropped.
    job_seq: u64,
    /// Cooperative cancel switch for the in-flight job, if any.
    cancel_flag: Option<Arc<AtomicBool>>,
    /// Streamed output tail from an in-progress/last CLI install run.
    pub install_log: Vec<String>,
    guide_queue: Vec<UnityAction>,
    guide_next_at: Option<Instant>,
    guide_kind: GuideKind,
    guide_total: usize,
    guide_genre: Option<GameGenre>,
    scaffold_save_path: String,
}

impl Default for UnityState {
    fn default() -> Self {
        let saved = load_unity_project_pref();
        let project_path = saved
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let mut state = Self {
            status: CliStatus::Unknown,
            cli_path: None,
            version_line: String::new(),
            last_error: None,
            project_path,
            editors_json: String::new(),
            editors_summary: "尚未拉取".into(),
            pipeline_status: PipelineStatus::Unknown,
            pipeline_summary: "尚未检查".into(),
            pipeline_detail: String::new(),
            editor_link: EditorLinkStatus::Unknown,
            commands_summary: "尚未探测".into(),
            eval_input: EVAL_PRESETS[0].1.into(),
            loop_object: "Ground".into(),
            busy: false,
            loop_phase: LoopPhase::Observe,
            demo_mode: false,
            scene: SceneSnapshot::default(),
            guide_label: None,
            toast: None,
            history: Vec::new(),
            next_id: 1,
            setup_step: SetupStep::InstallCli,
            setup_focus: None,
            project_locked: saved.is_some(),
            last_considered_cwd: None,
            pending_rx: None,
            job_seq: 0,
            cancel_flag: None,
            install_log: Vec::new(),
            guide_queue: Vec::new(),
            guide_next_at: None,
            guide_kind: GuideKind::None,
            guide_total: 0,
            guide_genre: None,
            scaffold_save_path: GameGenre::Playground.scene_path().into(),
        };
        if let Some(path) = saved {
            if let Some(root) = resolve_unity_project_root(&path) {
                state.project_path = root;
            }
        }
        state.sync_setup_step();
        state
    }
}

#[derive(Debug)]
enum UnityWorkerMsg {
    Detected {
        path: Option<PathBuf>,
        version: String,
        error: Option<String>,
    },
    CommandDone {
        action: UnityAction,
        title: String,
        command: String,
        phase: LoopPhase,
        ok: bool,
        stdout: String,
        stderr: String,
        elapsed_ms: u64,
    },
    InstallProgress {
        line: String,
    },
    InstallDone {
        ok: bool,
        message: String,
    },
}

impl UnityState {
    /// Spawn a background worker tagged with a fresh job id. Any prior
    /// in-flight job is asked to cancel (best-effort) and its late replies
    /// will be dropped by `drain_worker` since their id no longer matches
    /// `job_seq`. This is the single place that owns the thread+channel
    /// boilerplate previously duplicated across four call sites.
    fn spawn_job<F>(&mut self, work: F) -> Arc<AtomicBool>
    where
        F: FnOnce(u64, mpsc::Sender<(u64, UnityWorkerMsg)>, Arc<AtomicBool>) + Send + 'static,
    {
        if let Some(prev) = self.cancel_flag.take() {
            prev.store(true, Ordering::SeqCst);
        }
        self.job_seq += 1;
        let id = self.job_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.pending_rx = Some(rx);
        self.cancel_flag = Some(cancel.clone());
        let cancel_for_thread = cancel.clone();
        thread::spawn(move || work(id, tx, cancel_for_thread));
        cancel
    }

    /// Cooperatively cancel the in-flight job, if any. The worker thread
    /// notices the flag (checked inside `run_unity_timeout`'s wait loop or
    /// at each detect candidate) and kills the child process; the resulting
    /// message still carries the job id so it is accepted normally.
    pub fn cancel_active(&mut self) {
        if let Some(flag) = &self.cancel_flag {
            flag.store(true, Ordering::SeqCst);
            self.toast = Some("正在取消…".into());
        }
    }

    pub fn is_cancellable(&self) -> bool {
        self.busy && self.cancel_flag.is_some()
    }

    /// Stop everything: cancel the in-flight job (if cancellable) and clear
    /// any pending guided-demo queue so the wizard doesn't keep advancing.
    pub fn stop(&mut self) {
        self.cancel_active();
        if !self.guide_queue.is_empty() || self.guide_label.is_some() {
            self.guide_queue.clear();
            self.guide_next_at = None;
            self.guide_label = None;
            self.guide_kind = GuideKind::None;
            self.guide_total = 0;
            self.guide_genre = None;
            self.toast = Some("已停止".into());
        }
    }

    pub fn can_stop(&self) -> bool {
        self.busy || !self.guide_queue.is_empty() || self.guide_label.is_some()
    }

    pub fn ensure_detecting(&mut self) {
        if !matches!(self.status, CliStatus::Unknown) || self.busy {
            return;
        }
        self.status = CliStatus::Checking;
        self.busy = true;
        self.spawn_job(move |id, tx, cancel| {
            let result = detect_cli(Some(&cancel));
            let _ = tx.send((
                id,
                UnityWorkerMsg::Detected {
                    path: result.path,
                    version: result.version,
                    error: result.error,
                },
            ));
        });
    }

    /// Drain worker messages and advance guided demo queue.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.drain_worker();
        changed |= self.tick_guide();
        changed
    }

    fn drain_worker(&mut self) -> bool {
        let Some(rx) = self.pending_rx.as_ref() else {
            return false;
        };
        let mut msgs = Vec::new();
        while let Ok((id, msg)) = rx.try_recv() {
            if id == self.job_seq {
                msgs.push(msg);
            }
            // else: reply from a superseded job — silently dropped.
        }
        if msgs.is_empty() {
            return false;
        }
        self.cancel_flag = None;
        for msg in msgs {
            match msg {
                UnityWorkerMsg::Detected {
                    path,
                    version,
                    error,
                } => {
                    self.busy = false;
                    self.cli_path = path.clone();
                    self.version_line = version;
                    self.last_error = error.clone();
                    if path.is_some() {
                        self.status = CliStatus::Ready;
                        self.demo_mode = false;
                        self.toast = Some("已检测到 Unity CLI".into());
                    } else if error.is_some() {
                        self.status = CliStatus::Error;
                        self.demo_mode = true;
                        self.toast = Some("CLI 异常，已切换演示模式".into());
                    } else {
                        self.status = CliStatus::Missing;
                        self.demo_mode = true;
                        self.toast = Some("未安装 CLI，已切换演示模式".into());
                    }
                    self.sync_setup_step();
                }
                UnityWorkerMsg::CommandDone {
                    action,
                    title,
                    command,
                    phase,
                    ok,
                    stdout,
                    stderr,
                    elapsed_ms,
                } => {
                    self.busy = false;
                    let detail = merge_streams(&stdout, &stderr);
                    let summary = if ok {
                        match action {
                            UnityAction::ListEditors => summarize_editors_json(&stdout),
                            UnityAction::Eval
                            | UnityAction::SaveScene
                            | UnityAction::RefreshAssets
                            | UnityAction::RequestScriptReload
                            | UnityAction::ClearConsole
                            | UnityAction::PausePlayMode
                            | UnityAction::StepPlayMode
                            | UnityAction::UndoLast
                            | UnityAction::RedoLast
                            | UnityAction::FrameSelection
                            | UnityAction::FocusGameView
                            | UnityAction::FocusSceneView
                            | UnityAction::DuplicateSelection
                            | UnityAction::DeleteSelection
                            | UnityAction::ListScenes
                            | UnityAction::NewScene
                            | UnityAction::LoadFirstScene
                            | UnityAction::HierarchyRoots
                            | UnityAction::ActiveSceneInfo
                            | UnityAction::CreatePlane
                            | UnityAction::CreateDirectionalLight
                            | UnityAction::SelectLoopObject
                            | UnityAction::SetupSkyDay
                            | UnityAction::SetupSkySunset
                            | UnityAction::SetupSkyNight
                            | UnityAction::CreateGround
                            | UnityAction::SetupMainCamera
                            | UnityAction::CreatePlayerCapsule
                            | UnityAction::CreateNpc
                            | UnityAction::CreateNpcVendor
                            | UnityAction::CreateNpcQuest
                            | UnityAction::CreateSpawnPoint
                            | UnityAction::CreatePortalZone
                            | UnityAction::CreateEnemySpawn
                            | UnityAction::InstallNpcAi
                            | UnityAction::AttachNpcAi
                            | UnityAction::LayoutRpg
                            | UnityAction::LayoutMmo
                            | UnityAction::LayoutRoguelike
                            | UnityAction::SaveNamedScene
                            | UnityAction::SaveAssets
                            | UnityAction::FindAssets
                            | UnityAction::ConsoleErrors
                            | UnityAction::FindMissingScripts
                            | UnityAction::ListPackages
                            | UnityAction::AddPackage
                            | UnityAction::BuildWindowsPlayer => summarize_eval_output(&stdout),
                            UnityAction::EditorStatus => summarize_status_json(&stdout),
                            UnityAction::ListProjects => summarize_projects_json(&stdout),
                            UnityAction::ListLtsReleases => summarize_releases_json(&stdout),
                            UnityAction::RunEditModeTests | UnityAction::RunPlayModeTests => {
                                summarize_test_output(action, &stdout, &stderr)
                            }
                            UnityAction::Doctor
                            | UnityAction::EnvInfo
                            | UnityAction::LicenseInfo
                            | UnityAction::ProjectInfo
                            | UnityAction::HubLogs
                            | UnityAction::CacheInfo => truncate_one_line(&stdout, 160),
                            _ => truncate_one_line(&stdout, 120),
                        }
                    } else {
                        truncate_one_line(if stderr.is_empty() { &stdout } else { &stderr }, 120)
                    };
                    self.push_record(OpRecord {
                        id: self.next_id,
                        title,
                        command,
                        phase,
                        ok,
                        summary,
                        detail,
                        at_unix: now_unix(),
                        elapsed_ms,
                    });
                    self.next_id += 1;
                    if ok {
                        self.apply_action_effects(action, phase, &stdout);
                        // Scene-driven actions already set the highlight phase.
                        if !matches!(
                            action,
                            UnityAction::ObserveCollider
                                | UnityAction::FixCollider
                                | UnityAction::EnterPlayMode
                        ) {
                            self.loop_phase = phase.next();
                        }
                    } else {
                        self.apply_action_failure(action, &stderr, &stdout);
                    }
                    self.sync_setup_step();
                    if self.guide_queue.is_empty() {
                        if self.guide_label.is_some() {
                            let done = match self.guide_kind {
                                GuideKind::Scaffold => self
                                    .guide_genre
                                    .unwrap_or(GameGenre::Playground)
                                    .done_toast(),
                                GuideKind::NpcAi => "NPC AI 接入完成 ✓（进 Play 靠近 NPC 按 E 对话）",
                                _ => "完整闭环完成 ✓",
                            };
                            self.toast = Some(done.into());
                        }
                        self.guide_label = None;
                        self.guide_kind = GuideKind::None;
                        self.guide_total = 0;
                        self.guide_genre = None;
                    } else {
                        let gap = if self.guide_kind == GuideKind::NpcAi
                            && self.guide_queue.first() == Some(&UnityAction::AttachNpcAi)
                        {
                            Duration::from_secs(4)
                        } else {
                            GUIDE_STEP_GAP
                        };
                        self.guide_next_at = Some(Instant::now() + gap);
                    }
                }
                UnityWorkerMsg::InstallProgress { line } => {
                    push_install_log(&mut self.install_log, line);
                }
                UnityWorkerMsg::InstallDone { ok, message } => {
                    self.busy = false;
                    if ok {
                        push_install_log(&mut self.install_log, message);
                        self.toast = Some("Unity CLI 安装完成，正在重新检测…".into());
                        self.status = CliStatus::Unknown;
                        self.ensure_detecting();
                    } else {
                        self.status = CliStatus::Missing;
                        self.last_error = Some(message.clone());
                        push_install_log(&mut self.install_log, message);
                        self.toast = Some("Unity CLI 安装失败".into());
                    }
                }
            }
        }
        if !self.busy {
            self.pending_rx = None;
        }
        true
    }

    fn tick_guide(&mut self) -> bool {
        if self.busy || self.guide_queue.is_empty() {
            return false;
        }
        if let Some(at) = self.guide_next_at {
            if Instant::now() < at {
                return false;
            }
        }
        let next = self.guide_queue.remove(0);
        let prefix = match self.guide_kind {
            GuideKind::Scaffold => "创作",
            GuideKind::NpcAi => "NPC AI",
            _ => "闭环",
        };
        self.guide_label = Some(format!(
            "{prefix} {}/{} · {}",
            self.guide_step_index(),
            self.guide_total_steps(),
            next.label()
        ));
        self.guide_next_at = None;
        self.run_action(next);
        true
    }

    fn guide_total_steps(&self) -> usize {
        if self.guide_total > 0 {
            self.guide_total
        } else {
            self.guide_queue.len() + 1
        }
    }

    fn guide_step_index(&self) -> usize {
        let total = self.guide_total_steps();
        total.saturating_sub(self.guide_queue.len())
    }

    fn apply_action_effects(&mut self, action: UnityAction, phase: LoopPhase, stdout: &str) {
        let trimmed = stdout.trim();
        match action {
            UnityAction::ListEditors => {
                self.editors_json = trimmed.to_string();
                self.editors_summary = summarize_editors_json(trimmed);
            }
            UnityAction::ListPipeline => {
                self.pipeline_detail = trimmed.to_string();
                self.pipeline_summary = summarize_pipeline_list(trimmed);
                self.pipeline_status = infer_pipeline_status(trimmed, true);
                // The list response already contains the editor server state.
                // Use it directly; preview CLI table columns are not fixed.
                if let Some(reachable) = pipeline_server_reachable(trimmed) {
                    self.editor_link = if reachable {
                        EditorLinkStatus::Connected
                    } else {
                        EditorLinkStatus::Disconnected
                    };
                    self.commands_summary = if reachable {
                        "Pipeline 服务可达，可以执行 command / eval".into()
                    } else {
                        "Pipeline 包已登记，但编辑器服务不可达".into()
                    };
                }
                if pipeline_declared(&self.project_path)
                    && !pipeline_loaded_by_editor(&self.project_path)
                {
                    self.pipeline_status = PipelineStatus::PendingImport;
                    self.pipeline_summary =
                        "已写入 manifest，但当前 Editor 尚未解析；请完全关闭并重新打开工程".into();
                }
            }
            UnityAction::InstallPipeline => {
                self.pipeline_detail = trimmed.to_string();
                self.pipeline_summary = truncate_one_line(trimmed, 160);
                self.pipeline_status = if trimmed.is_empty() {
                    PipelineStatus::Installed
                } else {
                    infer_pipeline_status(trimmed, true)
                };
                if self.pipeline_status == PipelineStatus::Installed {
                    self.toast = Some("Pipeline 已安装，请等编辑器重编译后再探测".into());
                }
                if pipeline_declared(&self.project_path)
                    && !pipeline_loaded_by_editor(&self.project_path)
                {
                    self.pipeline_status = PipelineStatus::PendingImport;
                    self.pipeline_summary =
                        "已写入 manifest；请完全关闭并重新打开 Unity 工程".into();
                }
            }
            UnityAction::ListCommands | UnityAction::ProbeEditor => {
                self.commands_summary = truncate_one_line(trimmed, 160);
                self.editor_link = infer_editor_link(trimmed, true);
                // Only infer Pipeline from a real editor link, not demo chatter.
                if !self.demo_mode
                    && self.editor_link == EditorLinkStatus::Connected
                    && self.pipeline_status != PipelineStatus::Installed
                {
                    self.pipeline_status = PipelineStatus::Installed;
                    self.pipeline_summary = "编辑器已响应 command（视为已安装）".into();
                }
            }
            UnityAction::ObserveCollider => {
                self.scene.ground_collider_enabled = false;
                self.scene.is_playing = true;
                self.scene.player_y = -2.4;
                self.scene.last_eval_result = "false".into();
                self.scene.note = format!(
                    "观察：{} 的 Collider 被禁用，玩家掉出地板",
                    self.loop_object
                );
                self.loop_phase = LoopPhase::Act;
            }
            UnityAction::FixCollider => {
                self.scene.ground_collider_enabled = true;
                self.scene.last_eval_result = "true".into();
                self.scene.note = format!("行动：已重新启用 {} 的 Collider", self.loop_object);
                self.loop_phase = LoopPhase::Verify;
            }
            UnityAction::EnterPlayMode => {
                self.scene.is_playing = true;
                if self.scene.ground_collider_enabled {
                    self.scene.player_y = 1.0;
                    self.scene.note = "验证：Play Mode 中玩家站在地板上".into();
                } else {
                    self.scene.player_y = -2.4;
                    self.scene.note = "验证：碰撞体仍关闭，玩家掉落".into();
                }
                self.scene.last_eval_result = "true".into();
            }
            UnityAction::ExitPlayMode => {
                self.scene.is_playing = false;
                self.scene.note = "已退出 Play Mode".into();
                self.scene.last_eval_result = "false".into();
            }
            UnityAction::Eval
            | UnityAction::SaveScene
            | UnityAction::RefreshAssets
            | UnityAction::RequestScriptReload
            | UnityAction::ClearConsole
            | UnityAction::PausePlayMode
            | UnityAction::StepPlayMode
            | UnityAction::UndoLast
            | UnityAction::RedoLast
            | UnityAction::FrameSelection
            | UnityAction::FocusGameView
            | UnityAction::FocusSceneView
            | UnityAction::DuplicateSelection
            | UnityAction::DeleteSelection
            | UnityAction::ListScenes
            | UnityAction::NewScene
            | UnityAction::LoadFirstScene
            | UnityAction::HierarchyRoots
            | UnityAction::ActiveSceneInfo
            | UnityAction::CreatePlane
            | UnityAction::CreateDirectionalLight
            | UnityAction::SelectLoopObject
            | UnityAction::SetupSkyDay
            | UnityAction::SetupSkySunset
            | UnityAction::SetupSkyNight
            | UnityAction::CreateGround
            | UnityAction::SetupMainCamera
            | UnityAction::CreatePlayerCapsule
            | UnityAction::CreateNpc
            | UnityAction::CreateNpcVendor
            | UnityAction::CreateNpcQuest
            | UnityAction::CreateSpawnPoint
            | UnityAction::CreatePortalZone
            | UnityAction::CreateEnemySpawn
            | UnityAction::InstallNpcAi
            | UnityAction::AttachNpcAi
            | UnityAction::LayoutRpg
            | UnityAction::LayoutMmo
            | UnityAction::LayoutRoguelike
            | UnityAction::SaveNamedScene
            | UnityAction::SaveAssets
            | UnityAction::FindAssets
            | UnityAction::ConsoleErrors
            | UnityAction::FindMissingScripts
            | UnityAction::ListPackages
            | UnityAction::AddPackage
            | UnityAction::BuildWindowsPlayer => {
                self.scene.last_eval_result = truncate_one_line(trimmed, 80);
                self.scene.note = format!("{} 完成（{}）", action.label(), phase.label());
                if !trimmed.is_empty() && !trimmed.to_lowercase().contains("error") {
                    self.editor_link = EditorLinkStatus::Connected;
                }
                if matches!(action, UnityAction::PausePlayMode | UnityAction::StepPlayMode) {
                    self.scene.is_playing = true;
                }
                if matches!(action, UnityAction::CreateGround) {
                    self.scene.ground_collider_enabled = true;
                    self.loop_object = "Ground".into();
                }
                if matches!(
                    action,
                    UnityAction::CreatePlayerCapsule
                        | UnityAction::LayoutRpg
                        | UnityAction::LayoutMmo
                        | UnityAction::LayoutRoguelike
                ) {
                    self.scene.player_y = 1.1;
                }
                if matches!(action, UnityAction::BuildWindowsPlayer) {
                    self.toast = Some(truncate_one_line(trimmed, 160));
                }
            }
            UnityAction::EditorStatus => {
                self.commands_summary = summarize_status_json(trimmed);
                if let Some(connected) = status_has_instances(trimmed) {
                    self.editor_link = if connected {
                        EditorLinkStatus::Connected
                    } else {
                        EditorLinkStatus::Disconnected
                    };
                }
            }
            UnityAction::ListProjects => {
                self.toast = Some(summarize_projects_json(trimmed));
            }
            UnityAction::OpenProject => {
                self.toast = Some("已请求打开 Unity 工程".into());
            }
            UnityAction::RequireEditor => {
                self.toast = Some(truncate_one_line(trimmed, 120));
            }
            UnityAction::UpgradePipeline => {
                self.pipeline_detail = trimmed.to_string();
                self.pipeline_summary = truncate_one_line(trimmed, 160);
                self.toast = Some("Pipeline 升级完成，请等编辑器重编译".into());
            }
            UnityAction::RegisterProject | UnityAction::PinProject => {
                self.toast = Some(truncate_one_line(trimmed, 120));
            }
            UnityAction::ProjectInfo
            | UnityAction::ListLtsReleases
            | UnityAction::HubLogs
            | UnityAction::CacheInfo => {
                self.toast = Some(truncate_one_line(trimmed, 120));
            }
            UnityAction::RunEditModeTests | UnityAction::RunPlayModeTests => {
                self.toast = Some(summarize_test_output(action, trimmed, ""));
            }
            UnityAction::Doctor | UnityAction::EnvInfo | UnityAction::LicenseInfo => {
                self.toast = Some(truncate_one_line(trimmed, 120));
            }
            UnityAction::RefreshDetect
            | UnityAction::RunFullLoop
            | UnityAction::ScaffoldMiniGame
            | UnityAction::ScaffoldRpg
            | UnityAction::ScaffoldMmo
            | UnityAction::ScaffoldRoguelike
            | UnityAction::EnableNpcAi => {}
        }
    }

    fn apply_action_failure(&mut self, action: UnityAction, stderr: &str, stdout: &str) {
        let msg = truncate_one_line(
            if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            },
            160,
        );
        match action {
            UnityAction::InstallPipeline | UnityAction::UpgradePipeline => {
                self.pipeline_status = PipelineStatus::Error;
                self.pipeline_summary = msg.clone();
                self.pipeline_detail = merge_streams(stdout, stderr);
                self.toast = Some(format!("{}失败：{msg}", action.label()));
            }
            UnityAction::ListPipeline => {
                self.pipeline_status = PipelineStatus::Error;
                self.pipeline_summary = msg;
            }
            UnityAction::ListCommands
            | UnityAction::ProbeEditor
            | UnityAction::Eval
            | UnityAction::SaveScene
            | UnityAction::RefreshAssets
            | UnityAction::RequestScriptReload
            | UnityAction::ClearConsole
            | UnityAction::PausePlayMode
            | UnityAction::StepPlayMode
            | UnityAction::UndoLast
            | UnityAction::RedoLast
            | UnityAction::FrameSelection
            | UnityAction::FocusGameView
            | UnityAction::FocusSceneView
            | UnityAction::DuplicateSelection
            | UnityAction::DeleteSelection
            | UnityAction::ListScenes
            | UnityAction::NewScene
            | UnityAction::LoadFirstScene
            | UnityAction::HierarchyRoots
            | UnityAction::ActiveSceneInfo
            | UnityAction::CreatePlane
            | UnityAction::CreateDirectionalLight
            | UnityAction::SelectLoopObject
            | UnityAction::SetupSkyDay
            | UnityAction::SetupSkySunset
            | UnityAction::SetupSkyNight
            | UnityAction::CreateGround
            | UnityAction::SetupMainCamera
            | UnityAction::CreatePlayerCapsule
            | UnityAction::CreateNpc
            | UnityAction::CreateNpcVendor
            | UnityAction::CreateNpcQuest
            | UnityAction::CreateSpawnPoint
            | UnityAction::CreatePortalZone
            | UnityAction::CreateEnemySpawn
            | UnityAction::InstallNpcAi
            | UnityAction::AttachNpcAi
            | UnityAction::LayoutRpg
            | UnityAction::LayoutMmo
            | UnityAction::LayoutRoguelike
            | UnityAction::SaveNamedScene
            | UnityAction::SaveAssets
            | UnityAction::FindAssets
            | UnityAction::ConsoleErrors
            | UnityAction::FindMissingScripts
            | UnityAction::ListPackages
            | UnityAction::AddPackage
            | UnityAction::BuildWindowsPlayer => {
                self.editor_link = EditorLinkStatus::Disconnected;
                self.commands_summary = msg.clone();
                if msg.to_lowercase().contains("pipeline") {
                    self.toast = Some("编辑器未响应：请先安装 Pipeline 并打开项目".into());
                }
            }
            UnityAction::EditorStatus => {
                self.editor_link = EditorLinkStatus::Disconnected;
                self.commands_summary = msg;
            }
            _ => {}
        }
    }

    fn push_record(&mut self, record: OpRecord) {
        self.history.insert(0, record);
        if self.history.len() > 40 {
            self.history.truncate(40);
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.toast = Some("已清空操作时间线".into());
    }

    pub fn latest_record_id(&self) -> u64 {
        self.history.first().map(|record| record.id).unwrap_or(0)
    }

    pub fn latest_chat_result_since(&self, previous_id: u64) -> String {
        match self
            .history
            .first()
            .filter(|record| record.id != previous_id)
        {
            Some(record) if record.ok => format!(
                "Unity 操作已完成：{}。{}（{} ms）",
                record.title, record.summary, record.elapsed_ms
            ),
            Some(record) => format!("Unity 操作失败：{}。{}", record.title, record.summary),
            None => format!(
                "Unity 状态已刷新：CLI {}，Pipeline {}，编辑器{}。",
                self.status.label(),
                self.pipeline_status.label(),
                self.editor_link.label()
            ),
        }
    }

    pub fn reset_scene(&mut self) {
        self.scene = SceneSnapshot::default();
        self.loop_phase = LoopPhase::Observe;
        self.guide_queue.clear();
        self.guide_label = None;
        self.guide_kind = GuideKind::None;
        self.guide_total = 0;
        self.guide_genre = None;
        self.toast = Some("场景快照已重置".into());
    }

    pub fn chat_briefing(&self) -> String {
        let mode = if self.demo_mode { "演示" } else { "实机" };
        let latest = self
            .history
            .first()
            .map(|r| format!("{} · {} · {}", r.phase.label(), r.title, r.summary))
            .unwrap_or_else(|| "尚无操作记录".into());
        let project_ok = looks_like_unity_project(&self.project_path);

        // NEVER instruct the coding agent to shell out to `unity`.
        // Missing CLI, wrong cwd (task worktrees), or long pipeline installs all hang the UI.
        format!(
            "【只读分析，禁止执行任何 unity / pipeline / command eval 终端命令】\n\
             \n\
             当前 Unity 面板状态（{mode}）：\n\
             - CLI: {}\n\
             - 绑定工程: {}{}\n\
             - Pipeline: {} · {}\n\
             - 编辑器连接: {}\n\
             - 阶段: {}\n\
             - 场景: {}\n\
             - 备注: {}\n\
             - 最近面板操作: {}\n\
             \n\
             请只向用户解释现状与下一步；所有安装/探测/闭环必须让用户在侧栏「Unity CLI」引导页点按钮完成，\
             不要使用 run_terminal_cmd、bash、Shell 调用 unity。",
            self.status.label(),
            self.project_path.display(),
            if project_ok {
                ""
            } else {
                "（非 Unity 工程根；聊天任务目录无效）"
            },
            self.pipeline_status.label(),
            self.pipeline_summary,
            self.editor_link.label(),
            self.loop_phase.label(),
            self.scene.status_line(),
            self.scene.note,
            latest,
        )
    }

    /// Agent-only context for the "analyze in chat" action. This text is not
    /// rendered in the timeline; the user sees a short intent label instead.
    pub fn compact_chat_briefing(&self) -> String {
        let latest = self
            .history
            .first()
            .map(|record| format!("{} / {}", record.title, record.summary))
            .unwrap_or_else(|| "无".into());
        format!(
            "你正在分析 Bony Build 的 Unity 面板快照。只读分析，禁止调用终端、unity、pipeline、command 或 eval。\n\
             状态：CLI={}；项目={}；Pipeline={}（{}）；编辑器={}；阶段={}；场景={}；最近操作={}。\n\
             回答规则：不要逐项复述快照；先用一句话给出结论，再给最多 3 条可操作建议；\
             若状态正常，明确说“连接正常”，不要建议重启、重装或登录；总计不超过 120 个汉字。",
            self.status.label(),
            display_path(self.project_path.clone()).display(),
            self.pipeline_status.label(),
            self.pipeline_summary,
            self.editor_link.label(),
            self.loop_phase.label(),
            self.scene.status_line(),
            latest,
        )
    }

    pub fn set_project_path(&mut self, path: PathBuf) {
        let resolved = resolve_unity_project_root(&path).unwrap_or_else(|| path.clone());
        if self.project_path == resolved {
            return;
        }
        if resolved != path {
            self.toast = Some(format!(
                "已自动定位工程根：{}（原路径在子目录内）",
                resolved.display()
            ));
        }
        self.project_path = resolved.clone();
        self.project_locked = looks_like_unity_project(&resolved);
        self.last_considered_cwd = None; // force re-eval next consider
        if self.project_locked {
            save_unity_project_pref(&resolved);
        }
        if self.pipeline_status == PipelineStatus::Installed {
            self.pipeline_status = PipelineStatus::Unknown;
            self.pipeline_summary = "项目已切换，请刷新 Pipeline".into();
        }
        self.editor_link = EditorLinkStatus::Unknown;
        self.sync_setup_step();
    }

    /// Optionally adopt agent cwd only when it resolves to a Unity project and
    /// the user has not locked a dedicated Unity root.
    ///
    /// IMPORTANT: must be cheap to call. Callers used to invoke this every egui
    /// frame; `Path::canonicalize` + directory probes on Windows easily cost
    /// several milliseconds and made the whole UI feel like a slideshow.
    pub fn consider_agent_cwd(&mut self, cwd: &PathBuf) {
        // Fast path: locked Unity root — trust the lock, no disk I/O.
        if self.project_locked {
            return;
        }
        // Fast path: same cwd as last evaluation — skip canonicalize walk.
        if self.last_considered_cwd.as_ref() == Some(cwd) {
            return;
        }
        self.last_considered_cwd = Some(cwd.clone());

        if let Some(root) = resolve_unity_project_root(cwd) {
            self.set_project_path(root);
            return;
        }
        // Agent worktree / non-Unity cwd: keep existing path unless empty/missing.
        if self.project_path.as_os_str().is_empty() || !self.project_path.exists() {
            self.project_path = cwd.clone();
            self.project_locked = false;
            self.sync_setup_step();
        }
    }

    /// Recompute the recommended onboarding step from live state.
    pub fn sync_setup_step(&mut self) {
        let next = if self.status != CliStatus::Ready {
            SetupStep::InstallCli
        } else if !looks_like_unity_project(&self.project_path) {
            SetupStep::PickProject
        } else if self.pipeline_status != PipelineStatus::Installed
            && self.editor_link != EditorLinkStatus::Connected
        {
            SetupStep::InstallPipeline
        } else if self.editor_link != EditorLinkStatus::Connected {
            SetupStep::ProbeEditor
        } else {
            SetupStep::RunLoop
        };
        self.setup_step = next;
    }

    pub fn focused_setup_step(&self) -> SetupStep {
        self.setup_focus.unwrap_or(self.setup_step)
    }

    pub fn step_state(&self, step: SetupStep) -> StepState {
        let cur = self.setup_step.index();
        let idx = step.index();
        if idx < cur {
            StepState::Done
        } else if idx == cur {
            StepState::Current
        } else {
            StepState::Locked
        }
    }

    pub fn run_setup_primary(&mut self) {
        match self.focused_setup_step() {
            SetupStep::InstallCli => {
                // Copy is handled in UI; here we re-detect after user installs.
                self.run_action(UnityAction::RefreshDetect);
            }
            SetupStep::DetectCli => self.run_action(UnityAction::RefreshDetect),
            SetupStep::PickProject => {
                if looks_like_unity_project(&self.project_path) {
                    self.toast = Some("Unity 工程已确认，进入下一步".into());
                    self.sync_setup_step();
                } else {
                    self.toast = Some(
                        "请点「选择 Unity 工程根目录」选含 Assets 的文件夹（不要用 task worktree）"
                            .into(),
                    );
                }
            }
            SetupStep::InstallPipeline => self.run_action(UnityAction::InstallPipeline),
            SetupStep::ProbeEditor => self.run_action(UnityAction::ProbeEditor),
            SetupStep::RunLoop => self.run_action(UnityAction::RunFullLoop),
        }
    }

    pub fn advance_after_cli_install_copied(&mut self) {
        self.toast = Some("安装命令已复制。在 PowerShell 执行后点「我已安装，重新检测」".into());
        self.setup_focus = Some(SetupStep::DetectCli);
    }

    pub fn pipeline_ready_for_commands(&self) -> bool {
        matches!(self.editor_link, EditorLinkStatus::Connected)
    }

    pub fn checklist(&self) -> Vec<(&'static str, bool, String)> {
        vec![
            (
                "Unity CLI",
                self.status == CliStatus::Ready,
                if self.status == CliStatus::Ready {
                    "已就绪".into()
                } else {
                    self.status.label().into()
                },
            ),
            (
                "项目路径",
                looks_like_unity_project(&self.project_path),
                if looks_like_unity_project(&self.project_path) {
                    self.project_path.display().to_string()
                } else {
                    format!(
                        "{}（非 Unity 工程根，请在引导里重新选择）",
                        self.project_path.display()
                    )
                },
            ),
            (
                "Pipeline 包",
                self.pipeline_status == PipelineStatus::Installed,
                format!(
                    "{} · {}",
                    self.pipeline_status.label(),
                    self.pipeline_summary
                ),
            ),
            (
                "编辑器响应",
                self.editor_link == EditorLinkStatus::Connected,
                format!("{} · {}", self.editor_link.label(), self.commands_summary),
            ),
        ]
    }

    pub fn run_action(&mut self, action: UnityAction) {
        if self.busy
            && !matches!(
                action,
                UnityAction::RunFullLoop
                    | UnityAction::ScaffoldMiniGame
                    | UnityAction::ScaffoldRpg
                    | UnityAction::ScaffoldMmo
                    | UnityAction::ScaffoldRoguelike
                    | UnityAction::EnableNpcAi
            )
        {
            return;
        }

        if matches!(action, UnityAction::RunFullLoop) {
            self.start_full_loop();
            return;
        }
        if matches!(action, UnityAction::EnableNpcAi) {
            self.start_enable_npc_ai();
            return;
        }
        if let Some(genre) = match action {
            UnityAction::ScaffoldMiniGame => Some(GameGenre::Playground),
            UnityAction::ScaffoldRpg => Some(GameGenre::Rpg),
            UnityAction::ScaffoldMmo => Some(GameGenre::Mmo),
            UnityAction::ScaffoldRoguelike => Some(GameGenre::Roguelike),
            _ => None,
        } {
            self.start_scaffold(genre);
            return;
        }

        if matches!(action, UnityAction::RefreshDetect) {
            self.status = CliStatus::Unknown;
            self.busy = false;
            self.pending_rx = None;
            self.ensure_detecting();
            return;
        }

        if matches!(
            action,
            UnityAction::InstallPipeline | UnityAction::UpgradePipeline
        ) {
            self.pipeline_status = PipelineStatus::Installing;
            self.pipeline_summary = if matches!(action, UnityAction::UpgradePipeline) {
                "正在执行 unity pipeline upgrade…".into()
            } else {
                "正在执行 unity pipeline install…".into()
            };
        }
        if matches!(action, UnityAction::ListPipeline) {
            self.pipeline_status = PipelineStatus::Checking;
        }
        if matches!(action, UnityAction::ListCommands | UnityAction::ProbeEditor) {
            self.editor_link = EditorLinkStatus::Checking;
        }

        if matches!(action, UnityAction::HubLogs) {
            let path = hub_logs_dir_display();
            self.busy = true;
            self.spawn_job(move |id, tx, _cancel| {
                thread::sleep(Duration::from_millis(40));
                let _ = tx.send((
                    id,
                    UnityWorkerMsg::CommandDone {
                        action: UnityAction::HubLogs,
                        title: "Hub 日志".into(),
                        command: format!("open {path}"),
                        phase: LoopPhase::Observe,
                        ok: true,
                        stdout: format!("Hub logs directory:\n{path}\n"),
                        stderr: String::new(),
                        elapsed_ms: 5,
                    },
                ));
            });
            return;
        }

        if self.demo_mode || self.status != CliStatus::Ready {
            self.run_demo(action);
            return;
        }
        let Some(cli) = self.cli_path.clone() else {
            self.run_demo(action);
            return;
        };

        let project = self.project_path.clone();
        let (title, args, phase) = action.to_cli_args(
            &self.eval_input,
            &project,
            &self.loop_object,
            &self.scaffold_save_path,
        );
        let command_display = format_command(&cli, &args);
        let timeout = action.timeout();
        self.busy = true;
        self.spawn_job(move |id, tx, cancel| {
            let started = Instant::now();
            let result = run_unity_timeout(&cli, &args, timeout, Some(&project), Some(&cancel));
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let ok = result.ok
                && (!action.is_eval_style() || eval_output_succeeded(&result.stdout));
            let _ = tx.send((
                id,
                UnityWorkerMsg::CommandDone {
                    action,
                    title,
                    command: command_display,
                    phase,
                    ok,
                    stdout: result.stdout,
                    stderr: result.stderr,
                    elapsed_ms,
                },
            ));
        });
    }

    fn start_full_loop(&mut self) {
        if self.busy || !self.guide_queue.is_empty() {
            return;
        }
        self.demo_mode = self.status != CliStatus::Ready;
        self.scene = SceneSnapshot {
            player_y: 1.0,
            ground_collider_enabled: true,
            is_playing: false,
            last_eval_result: "—".into(),
            note: format!(
                "准备复现：检查对象「{}」的 Collider",
                self.loop_object
            ),
        };
        self.loop_phase = LoopPhase::Observe;
        self.guide_kind = GuideKind::Loop;
        self.guide_queue = vec![
            UnityAction::ObserveCollider,
            UnityAction::FixCollider,
            UnityAction::EnterPlayMode,
        ];
        self.guide_total = self.guide_queue.len();
        self.guide_label = Some(format!("闭环 0/{} · 准备中", self.guide_total));
        self.guide_next_at = Some(Instant::now());
        self.toast = Some("开始完整闭环演示".into());
    }

    fn start_scaffold(&mut self, genre: GameGenre) {
        if self.busy || !self.guide_queue.is_empty() {
            return;
        }
        self.demo_mode = self.status != CliStatus::Ready;
        self.loop_phase = LoopPhase::Act;
        self.guide_genre = Some(genre);
        self.scaffold_save_path = genre.scene_path().into();
        self.scene.note = format!(
            "开始搭建 {}：共用底座 → 类型布局 → {}",
            genre.label(),
            genre.scene_path()
        );
        self.guide_kind = GuideKind::Scaffold;
        let sky = match genre {
            GameGenre::Roguelike => UnityAction::SetupSkyNight,
            _ => UnityAction::SetupSkyDay,
        };
        let mut queue = vec![UnityAction::NewScene, sky];
        match genre {
            GameGenre::Playground => {
                queue.extend([
                    UnityAction::CreateGround,
                    UnityAction::CreateDirectionalLight,
                    UnityAction::SetupMainCamera,
                    UnityAction::CreatePlayerCapsule,
                ]);
            }
            GameGenre::Rpg => {
                queue.extend([
                    UnityAction::CreateGround,
                    UnityAction::CreateDirectionalLight,
                    UnityAction::SetupMainCamera,
                    UnityAction::LayoutRpg,
                ]);
            }
            GameGenre::Mmo => {
                queue.extend([
                    UnityAction::CreateDirectionalLight,
                    UnityAction::SetupMainCamera,
                    UnityAction::LayoutMmo,
                ]);
            }
            GameGenre::Roguelike => {
                queue.extend([
                    UnityAction::CreateDirectionalLight,
                    UnityAction::SetupMainCamera,
                    UnityAction::LayoutRoguelike,
                ]);
            }
        }
        queue.extend([UnityAction::SaveNamedScene, UnityAction::EnterPlayMode]);
        self.guide_queue = queue;
        self.guide_total = self.guide_queue.len();
        self.guide_label = Some(format!("创作 0/{} · 准备中", self.guide_total));
        self.guide_next_at = Some(Instant::now());
        self.toast = Some(genre.start_toast().into());
    }

    fn start_enable_npc_ai(&mut self) {
        if self.busy || !self.guide_queue.is_empty() {
            return;
        }
        self.demo_mode = self.status != CliStatus::Ready;
        self.loop_phase = LoopPhase::Act;
        self.guide_genre = None;
        self.scene.note =
            "接入 NPC AI：写入脚本 → 重编译 → 挂载到场景中所有 NPC_*".into();
        self.guide_kind = GuideKind::NpcAi;
        self.guide_queue = vec![
            UnityAction::InstallNpcAi,
            UnityAction::RequestScriptReload,
            UnityAction::AttachNpcAi,
        ];
        self.guide_total = self.guide_queue.len();
        self.guide_label = Some(format!("NPC AI 0/{} · 准备中", self.guide_total));
        self.guide_next_at = Some(Instant::now());
        self.toast = Some("开始给 NPC 接入 AI".into());
    }

    fn run_demo(&mut self, action: UnityAction) {
        let (title, phase, ok, summary, detail, command) = match action {
            UnityAction::ListEditors => (
                "列出已安装编辑器".into(),
                LoopPhase::Observe,
                true,
                "demo: 2 editors (6000.2.10f1, 6000.0.28f1)".into(),
                DEMO_EDITORS_JSON.into(),
                "unity editors --format json".into(),
            ),
            UnityAction::ListPipeline => (
                "刷新 Pipeline 列表".into(),
                LoopPhase::Observe,
                true,
                "demo: Pipeline: Installed".into(),
                format!(
                    "Project: {}\nPipeline: Installed (com.unity.pipeline)\n",
                    self.project_path.display()
                ),
                "unity pipeline list".into(),
            ),
            UnityAction::InstallPipeline => (
                "安装 com.unity.pipeline".into(),
                LoopPhase::Observe,
                true,
                "demo: Pipeline: Installed".into(),
                format!(
                    "Installing com.unity.pipeline into {}\nPipeline: Installed\nWait for Editor recompile, then run unity command.\n",
                    self.project_path.display()
                ),
                "unity pipeline install".into(),
            ),
            UnityAction::ListCommands => (
                "发现已注册命令".into(),
                LoopPhase::Observe,
                true,
                "demo: greet, eval, play, stop".into(),
                "greet — Log a greeting\neval — Evaluate C# in the Editor\nplay — Enter Play Mode\nstop — Exit Play Mode\n".into(),
                "unity command".into(),
            ),
            UnityAction::ProbeEditor => (
                "探测编辑器连接".into(),
                LoopPhase::Observe,
                true,
                "demo: editor connected · 4 commands".into(),
                "Connected to Editor\ngreet\neval\nplay\nstop\n".into(),
                "unity command".into(),
            ),
            UnityAction::Eval => {
                let expr = self.eval_input.clone();
                let fake = demo_eval_result(&expr, &self.scene);
                (
                    "Eval C# 表达式".into(),
                    LoopPhase::Act,
                    true,
                    format!("demo ← {fake}"),
                    format!("{{\n  \"ok\": true,\n  \"result\": {fake},\n  \"expr\": {expr:?}\n}}\n"),
                    format!("unity command eval {expr:?}"),
                )
            }
            UnityAction::ObserveCollider => (
                "观察：碰撞体已禁用".into(),
                LoopPhase::Observe,
                true,
                "demo: GroundCollider.enabled == false".into(),
                "Bug report: player sometimes falls through the floor.\nInspect: GameObject.Find(\"Ground\").GetComponent<Collider>().enabled → false\n".into(),
                "unity command eval \"return GameObject.Find(\\\"Ground\\\").GetComponent<Collider>().enabled;\"".into(),
            ),
            UnityAction::FixCollider => (
                "行动：重新启用碰撞体".into(),
                LoopPhase::Act,
                true,
                "demo: collider.enabled = true".into(),
                "Action: GroundCollider.enabled = true\nNo domain reload · eval returned true\n".into(),
                "unity command eval \"var c = GameObject.Find(\\\"Ground\\\").GetComponent<Collider>(); c.enabled = true; return c.enabled;\"".into(),
            ),
            UnityAction::EnterPlayMode => (
                "验证：进入 Play Mode".into(),
                LoopPhase::Verify,
                true,
                "demo: isPlaying = true · player stable".into(),
                "Enter Play Mode\nisPlaying → true\nPlayer remains on floor\n".into(),
                "unity command eval \"UnityEditor.EditorApplication.isPlaying = true; return UnityEditor.EditorApplication.isPlaying;\"".into(),
            ),
            UnityAction::ExitPlayMode => (
                "退出 Play Mode".into(),
                LoopPhase::Verify,
                true,
                "demo: isPlaying = false".into(),
                "{\n  \"isPlaying\": false\n}\n".into(),
                "unity command eval \"UnityEditor.EditorApplication.isPlaying = false; return UnityEditor.EditorApplication.isPlaying;\"".into(),
            ),
            UnityAction::SaveScene => (
                "保存场景".into(),
                LoopPhase::Act,
                true,
                "demo: SaveOpenScenes → true".into(),
                "{\n  \"ok\": true,\n  \"result\": true\n}\n".into(),
                "unity command eval SaveOpenScenes".into(),
            ),
            UnityAction::RefreshAssets => (
                "刷新资源".into(),
                LoopPhase::Act,
                true,
                "demo: AssetDatabase.Refresh".into(),
                "{\n  \"ok\": true,\n  \"result\": true\n}\n".into(),
                "unity command eval AssetDatabase.Refresh".into(),
            ),
            UnityAction::RequestScriptReload => (
                "重编译脚本".into(),
                LoopPhase::Act,
                true,
                "demo: RequestScriptReload".into(),
                "{\n  \"ok\": true,\n  \"result\": true\n}\n".into(),
                "unity command eval RequestScriptReload".into(),
            ),
            UnityAction::ClearConsole => (
                "清控制台".into(),
                LoopPhase::Act,
                true,
                "demo: console cleared".into(),
                "{\n  \"ok\": true,\n  \"result\": \"cleared\"\n}\n".into(),
                "unity command eval LogEntries.Clear".into(),
            ),
            UnityAction::PausePlayMode => (
                "暂停/继续 Play".into(),
                LoopPhase::Act,
                true,
                "demo: isPaused toggled".into(),
                "{\n  \"ok\": true,\n  \"result\": true\n}\n".into(),
                "unity command eval isPaused toggle".into(),
            ),
            UnityAction::StepPlayMode => (
                "单帧步进".into(),
                LoopPhase::Act,
                true,
                "demo: Step()".into(),
                "{\n  \"ok\": true,\n  \"result\": true\n}\n".into(),
                "unity command eval Step".into(),
            ),
            UnityAction::UndoLast => (
                "撤销".into(),
                LoopPhase::Act,
                true,
                "demo: undo".into(),
                "{\n  \"ok\": true,\n  \"result\": \"undo\"\n}\n".into(),
                "unity command eval Undo".into(),
            ),
            UnityAction::RedoLast => (
                "重做".into(),
                LoopPhase::Act,
                true,
                "demo: redo".into(),
                "{\n  \"ok\": true,\n  \"result\": \"redo\"\n}\n".into(),
                "unity command eval Redo".into(),
            ),
            UnityAction::FrameSelection => (
                "框选聚焦".into(),
                LoopPhase::Act,
                true,
                "demo: FrameLastActiveSceneView".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Player\"\n}\n".into(),
                "unity command eval Frame".into(),
            ),
            UnityAction::FocusGameView => (
                "切到 Game".into(),
                LoopPhase::Act,
                true,
                "demo: Game view".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Game\"\n}\n".into(),
                "unity command eval FocusGame".into(),
            ),
            UnityAction::FocusSceneView => (
                "切到 Scene".into(),
                LoopPhase::Act,
                true,
                "demo: Scene view".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Scene\"\n}\n".into(),
                "unity command eval FocusScene".into(),
            ),
            UnityAction::DuplicateSelection => (
                "复制选中".into(),
                LoopPhase::Act,
                true,
                "demo: duplicated 1".into(),
                "{\n  \"ok\": true,\n  \"result\": 1\n}\n".into(),
                "unity command eval Duplicate".into(),
            ),
            UnityAction::DeleteSelection => (
                "删除选中".into(),
                LoopPhase::Act,
                true,
                "demo: deleted 1".into(),
                "{\n  \"ok\": true,\n  \"result\": 1\n}\n".into(),
                "unity command eval Delete".into(),
            ),
            UnityAction::ListScenes => (
                "列构建场景".into(),
                LoopPhase::Observe,
                true,
                "demo: 2 build scenes".into(),
                "{\n  \"ok\": true,\n  \"result\": \"[x] Assets/Scenes/SampleScene.unity\"\n}\n".into(),
                "unity command eval ListScenes".into(),
            ),
            UnityAction::NewScene => (
                "新建场景".into(),
                LoopPhase::Act,
                true,
                "demo: new scene".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Untitled\"\n}\n".into(),
                "unity command eval NewScene".into(),
            ),
            UnityAction::LoadFirstScene => (
                "加载首场景".into(),
                LoopPhase::Act,
                true,
                "demo: loaded SampleScene".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Assets/Scenes/SampleScene.unity\"\n}\n".into(),
                "unity command eval LoadFirstScene".into(),
            ),
            UnityAction::HierarchyRoots => (
                "场景根物体".into(),
                LoopPhase::Observe,
                true,
                "demo: Main Camera, Directional Light".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Main Camera\\nDirectional Light\"\n}\n".into(),
                "unity command eval HierarchyRoots".into(),
            ),
            UnityAction::ActiveSceneInfo => (
                "当前场景".into(),
                LoopPhase::Observe,
                true,
                "demo: SampleScene".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Assets/Scenes/SampleScene.unity\"\n}\n".into(),
                "unity command eval ActiveScene".into(),
            ),
            UnityAction::CreatePlane => (
                "创建平面".into(),
                LoopPhase::Act,
                true,
                "demo: BonyPlane".into(),
                "{\n  \"ok\": true,\n  \"result\": \"BonyPlane\"\n}\n".into(),
                "unity command eval CreatePlane".into(),
            ),
            UnityAction::CreateDirectionalLight => (
                "创建平行光".into(),
                LoopPhase::Act,
                true,
                "demo: BonyDirectionalLight".into(),
                "{\n  \"ok\": true,\n  \"result\": \"BonyDirectionalLight\"\n}\n".into(),
                "unity command eval CreateLight".into(),
            ),
            UnityAction::SetupSkyDay => (
                "白天天空".into(),
                LoopPhase::Act,
                true,
                "demo: day sky".into(),
                "{\n  \"ok\": true,\n  \"result\": \"day\"\n}\n".into(),
                "unity command eval SetupSkyDay".into(),
            ),
            UnityAction::SetupSkySunset => (
                "晚霞天空".into(),
                LoopPhase::Act,
                true,
                "demo: sunset sky".into(),
                "{\n  \"ok\": true,\n  \"result\": \"sunset\"\n}\n".into(),
                "unity command eval SetupSkySunset".into(),
            ),
            UnityAction::SetupSkyNight => (
                "夜空".into(),
                LoopPhase::Act,
                true,
                "demo: night sky".into(),
                "{\n  \"ok\": true,\n  \"result\": \"night\"\n}\n".into(),
                "unity command eval SetupSkyNight".into(),
            ),
            UnityAction::CreateGround => (
                "创建地面".into(),
                LoopPhase::Act,
                true,
                "demo: Ground".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Ground scale=(8,1,8)\"\n}\n".into(),
                "unity command eval CreateGround".into(),
            ),
            UnityAction::SetupMainCamera => (
                "设置主相机".into(),
                LoopPhase::Act,
                true,
                "demo: Main Camera".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Main Camera @ (0,5,-10)\"\n}\n".into(),
                "unity command eval SetupMainCamera".into(),
            ),
            UnityAction::CreatePlayerCapsule => (
                "创建玩家".into(),
                LoopPhase::Act,
                true,
                "demo: Player".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Player y=1.1\"\n}\n".into(),
                "unity command eval CreatePlayer".into(),
            ),
            UnityAction::CreateNpc => (
                "创建 NPC".into(),
                LoopPhase::Act,
                true,
                "demo: NPC_1".into(),
                "{\n  \"ok\": true,\n  \"result\": \"NPC_1\"\n}\n".into(),
                "unity command eval CreateNpc".into(),
            ),
            UnityAction::CreateNpcVendor => (
                "创建商人 NPC".into(),
                LoopPhase::Act,
                true,
                "demo: NPC_Vendor".into(),
                "{\n  \"ok\": true,\n  \"result\": \"NPC_Vendor\"\n}\n".into(),
                "unity command eval CreateNpcVendor".into(),
            ),
            UnityAction::CreateNpcQuest => (
                "创建任务 NPC".into(),
                LoopPhase::Act,
                true,
                "demo: NPC_Quest".into(),
                "{\n  \"ok\": true,\n  \"result\": \"NPC_Quest\"\n}\n".into(),
                "unity command eval CreateNpcQuest".into(),
            ),
            UnityAction::CreateSpawnPoint => (
                "创建出生点".into(),
                LoopPhase::Act,
                true,
                "demo: Spawn_1".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Spawn_1\"\n}\n".into(),
                "unity command eval CreateSpawnPoint".into(),
            ),
            UnityAction::CreatePortalZone => (
                "创建传送门".into(),
                LoopPhase::Act,
                true,
                "demo: Portal_Zone".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Portal_Zone\"\n}\n".into(),
                "unity command eval CreatePortalZone".into(),
            ),
            UnityAction::CreateEnemySpawn => (
                "创建敌人点".into(),
                LoopPhase::Act,
                true,
                "demo: Enemy_Spawn_1".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Enemy_Spawn_1\"\n}\n".into(),
                "unity command eval CreateEnemySpawn".into(),
            ),
            UnityAction::InstallNpcAi => (
                "安装 NPC AI 脚本".into(),
                LoopPhase::Act,
                true,
                "demo: installed Assets/Bony/NpcAi".into(),
                "{\n  \"ok\": true,\n  \"result\": \"installed Assets/Bony/NpcAi (BonyNpcBrain + BonyNpcDialogue)\"\n}\n".into(),
                "unity command eval InstallNpcAi".into(),
            ),
            UnityAction::AttachNpcAi => (
                "挂载 NPC AI".into(),
                LoopPhase::Act,
                true,
                "demo: attached NPC AI".into(),
                "{\n  \"ok\": true,\n  \"result\": \"attached NPC AI to 2 objects\"\n}\n".into(),
                "unity command eval AttachNpcAi".into(),
            ),
            UnityAction::LayoutRpg => (
                "RPG 布局".into(),
                LoopPhase::Act,
                true,
                "demo: RPG layout".into(),
                "{\n  \"ok\": true,\n  \"result\": \"rpg layout: NPC_Vendor NPC_Quest Spawn_Town HUD\"\n}\n".into(),
                "unity command eval LayoutRpg".into(),
            ),
            UnityAction::LayoutMmo => (
                "MMO 布局".into(),
                LoopPhase::Act,
                true,
                "demo: MMO layout".into(),
                "{\n  \"ok\": true,\n  \"result\": \"mmo layout: World_Hub Spawn_A Spawn_B Spawn_C Portal_Zone ChatPanel MinimapFrame\"\n}\n".into(),
                "unity command eval LayoutMmo".into(),
            ),
            UnityAction::LayoutRoguelike => (
                "肉鸽布局".into(),
                LoopPhase::Act,
                true,
                "demo: Roguelike layout".into(),
                "{\n  \"ok\": true,\n  \"result\": \"roguelike layout: rooms Enemy_Spawn Door_North RunManager RunHUD\"\n}\n".into(),
                "unity command eval LayoutRoguelike".into(),
            ),
            UnityAction::SaveNamedScene => (
                "保存雏形场景".into(),
                LoopPhase::Act,
                true,
                format!("demo: {}", self.scaffold_save_path),
                format!(
                    "{{\n  \"ok\": true,\n  \"result\": \"{}\"\n}}\n",
                    self.scaffold_save_path
                ),
                "unity command eval SaveNamedScene".into(),
            ),
            UnityAction::SelectLoopObject => (
                "选中闭环对象".into(),
                LoopPhase::Observe,
                true,
                format!("demo: selected {}", self.loop_object),
                format!(
                    "{{\n  \"ok\": true,\n  \"result\": \"{}\"\n}}\n",
                    self.loop_object
                ),
                "unity command eval SelectLoop".into(),
            ),
            UnityAction::SaveAssets => (
                "保存资源".into(),
                LoopPhase::Act,
                true,
                "demo: SaveAssets".into(),
                "{\n  \"ok\": true,\n  \"result\": true\n}\n".into(),
                "unity command eval SaveAssets".into(),
            ),
            UnityAction::FindAssets => (
                "搜索资源".into(),
                LoopPhase::Observe,
                true,
                "demo: 3 prefabs".into(),
                "{\n  \"ok\": true,\n  \"result\": \"count=3\\nAssets/Prefabs/Player.prefab\"\n}\n".into(),
                "unity command eval FindAssets".into(),
            ),
            UnityAction::ConsoleErrors => (
                "控制台错误".into(),
                LoopPhase::Observe,
                true,
                "demo: errors=0".into(),
                "{\n  \"ok\": true,\n  \"result\": \"errors=0/0\\n\"\n}\n".into(),
                "unity command eval ConsoleErrors".into(),
            ),
            UnityAction::FindMissingScripts => (
                "缺失脚本".into(),
                LoopPhase::Observe,
                true,
                "demo: missing=0".into(),
                "{\n  \"ok\": true,\n  \"result\": \"missing=0\"\n}\n".into(),
                "unity command eval MissingScripts".into(),
            ),
            UnityAction::ListPackages => (
                "列出包".into(),
                LoopPhase::Observe,
                true,
                "demo: 12 packages".into(),
                "{\n  \"ok\": true,\n  \"result\": \"count=12\\ncom.unity.pipeline@1.0.0\"\n}\n".into(),
                "unity command eval ListPackages".into(),
            ),
            UnityAction::AddPackage => (
                "安装包".into(),
                LoopPhase::Act,
                true,
                "demo: package added".into(),
                "{\n  \"ok\": true,\n  \"result\": \"com.unity.ugui@2.0.0\"\n}\n".into(),
                "unity command eval AddPackage".into(),
            ),
            UnityAction::BuildWindowsPlayer => (
                "构建 Win64".into(),
                LoopPhase::Verify,
                true,
                "demo: Succeeded Builds/Win64/Player.exe".into(),
                "{\n  \"ok\": true,\n  \"result\": \"Succeeded C:/Demo/Builds/Win64/Player.exe\"\n}\n".into(),
                "unity command eval BuildWin64".into(),
            ),
            UnityAction::EditorStatus => (
                "编辑器状态".into(),
                LoopPhase::Observe,
                true,
                "demo: 0 connected editors".into(),
                DEMO_STATUS_JSON.into(),
                "unity status --format json".into(),
            ),
            UnityAction::ListProjects => (
                "Hub 工程列表".into(),
                LoopPhase::Observe,
                true,
                "demo: 2 Hub projects".into(),
                DEMO_PROJECTS_JSON.into(),
                "unity projects list --json".into(),
            ),
            UnityAction::OpenProject => (
                "打开工程".into(),
                LoopPhase::Act,
                true,
                format!("demo: open {}", self.project_path.display()),
                format!("Opening {}\n", self.project_path.display()),
                format!("unity open {}", self.project_path.display()),
            ),
            UnityAction::RequireEditor => (
                "补齐编辑器".into(),
                LoopPhase::Observe,
                true,
                "demo: required editor present".into(),
                "Editor version already installed.\n".into(),
                format!("unity projects require {}", self.project_path.display()),
            ),
            UnityAction::UpgradePipeline => (
                "升级 Pipeline".into(),
                LoopPhase::Observe,
                true,
                "demo: Pipeline upgraded".into(),
                "Pipeline package upgraded to latest.\n".into(),
                "unity pipeline upgrade".into(),
            ),
            UnityAction::ProjectInfo => (
                "工程信息".into(),
                LoopPhase::Observe,
                true,
                "demo: project info".into(),
                format!(
                    "{{\"title\":\"Demo\",\"path\":\"{}\"}}\n",
                    self.project_path.display()
                ),
                format!("unity projects info {}", self.project_path.display()),
            ),
            UnityAction::RegisterProject => (
                "注册到 Hub".into(),
                LoopPhase::Act,
                true,
                "demo: project registered".into(),
                "Registered project in Hub.\n".into(),
                format!("unity projects add {}", self.project_path.display()),
            ),
            UnityAction::PinProject => (
                "收藏工程".into(),
                LoopPhase::Act,
                true,
                "demo: project pinned".into(),
                "Pinned project in Hub.\n".into(),
                format!("unity projects pin {}", self.project_path.display()),
            ),
            UnityAction::ListLtsReleases => (
                "LTS 版本".into(),
                LoopPhase::Observe,
                true,
                "demo: 3 LTS releases".into(),
                DEMO_RELEASES_JSON.into(),
                "unity releases --lts --json --limit 10".into(),
            ),
            UnityAction::HubLogs => (
                "Hub 日志".into(),
                LoopPhase::Observe,
                true,
                "demo: hub logs path".into(),
                format!("Hub logs directory:\n{}\n", hub_logs_dir_display()),
                format!("open {}", hub_logs_dir_display()),
            ),
            UnityAction::CacheInfo => (
                "下载缓存".into(),
                LoopPhase::Observe,
                true,
                "demo: cache 1.2 GB".into(),
                "path: ~/.unity/cache\nsize: 1.2 GB\n".into(),
                "unity cache info".into(),
            ),
            UnityAction::RunEditModeTests => (
                "EditMode 测试".into(),
                LoopPhase::Verify,
                true,
                "demo: EditMode tests passed".into(),
                "EditMode: 0 failures\n".into(),
                format!(
                    "unity test {} --mode EditMode",
                    self.project_path.display()
                ),
            ),
            UnityAction::RunPlayModeTests => (
                "PlayMode 测试".into(),
                LoopPhase::Verify,
                true,
                "demo: PlayMode tests passed".into(),
                "PlayMode: 0 failures\n".into(),
                format!(
                    "unity test {} --mode PlayMode",
                    self.project_path.display()
                ),
            ),
            UnityAction::Doctor => (
                "环境诊断".into(),
                LoopPhase::Observe,
                true,
                "demo: doctor ok".into(),
                "CLI: ready\nHub: ready\n".into(),
                "unity doctor --format json".into(),
            ),
            UnityAction::EnvInfo => (
                "Hub 环境".into(),
                LoopPhase::Observe,
                true,
                "demo: env paths".into(),
                "hubPath: demo\ncliPath: demo\n".into(),
                "unity env --format json".into(),
            ),
            UnityAction::LicenseInfo => (
                "许可信息".into(),
                LoopPhase::Observe,
                true,
                "demo: license ok".into(),
                "licenses: [Personal]\n".into(),
                "unity license --format json".into(),
            ),
            UnityAction::RefreshDetect
            | UnityAction::RunFullLoop
            | UnityAction::ScaffoldMiniGame
            | UnityAction::ScaffoldRpg
            | UnityAction::ScaffoldMmo
            | UnityAction::ScaffoldRoguelike
            | UnityAction::EnableNpcAi => return,
        };

        self.demo_mode = true;
        self.busy = true;
        let stdout = if matches!(
            action,
            UnityAction::ListEditors
                | UnityAction::EditorStatus
                | UnityAction::ListProjects
                | UnityAction::ListLtsReleases
        ) {
            match action {
                UnityAction::ListEditors => DEMO_EDITORS_JSON.to_string(),
                UnityAction::EditorStatus => DEMO_STATUS_JSON.to_string(),
                UnityAction::ListProjects => DEMO_PROJECTS_JSON.to_string(),
                UnityAction::ListLtsReleases => DEMO_RELEASES_JSON.to_string(),
                _ => format!("{summary}\n{detail}"),
            }
        } else {
            format!("{summary}\n{detail}")
        };
        let action_copy = action;
        self.spawn_job(move |id, tx, _cancel| {
            thread::sleep(Duration::from_millis(180));
            let _ = tx.send((
                id,
                UnityWorkerMsg::CommandDone {
                    action: action_copy,
                    title,
                    command,
                    phase,
                    ok,
                    stdout,
                    stderr: String::new(),
                    elapsed_ms: 12,
                },
            ));
        });
    }

    pub fn install_hint() -> &'static str {
        if cfg!(windows) {
            Self::install_hint_windows()
        } else {
            Self::install_hint_unix()
        }
    }

    pub fn install_hint_windows() -> &'static str {
        "$env:UNITY_CLI_CHANNEL='beta'; irm https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.ps1 | iex"
    }

    pub fn install_hint_unix() -> &'static str {
        "curl -fsSL https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.sh | UNITY_CLI_CHANNEL=beta bash"
    }

    /// One-click install entry point for when the CLI isn't found locally.
    /// Runs the same install script `install_hint()` shows, streaming its
    /// output into `install_log`, then re-triggers detection on success.
    pub fn install_cli(&mut self) {
        if self.busy {
            return;
        }
        self.status = CliStatus::Installing;
        self.busy = true;
        self.install_log.clear();
        self.toast = Some("正在安装 Unity CLI…".into());
        self.spawn_job(move |id, tx, cancel| {
            let outcome = run_cli_install(id, &tx, &cancel);
            let msg = match outcome {
                Ok(()) => UnityWorkerMsg::InstallDone {
                    ok: true,
                    message: "Unity CLI 安装完成".into(),
                },
                Err(reason) => UnityWorkerMsg::InstallDone {
                    ok: false,
                    message: reason,
                },
            };
            let _ = tx.send((id, msg));
        });
    }

    pub fn can_install_cli(&self) -> bool {
        !self.busy && matches!(self.status, CliStatus::Missing | CliStatus::Error)
    }

    pub fn take_toast(&mut self) -> Option<String> {
        self.toast.take()
    }

    pub fn needs_repaint(&self) -> bool {
        self.busy || !self.guide_queue.is_empty() || self.guide_next_at.is_some()
    }

    pub fn is_guiding(&self) -> bool {
        !self.guide_queue.is_empty() || self.guide_label.is_some()
    }
}

impl UnityAction {
    fn to_cli_args(
        self,
        eval_input: &str,
        project: &PathBuf,
        loop_object: &str,
        save_scene_path: &str,
    ) -> (String, Vec<String>, LoopPhase) {
        // `Path::canonicalize` returns an extended-length `\\?\C:\...` path on
        // Windows. Unity Pipeline registers editor instances under the normal
        // DOS/UNC spelling, so passing the extended spelling makes the CLI
        // miss an otherwise running editor.
        let project_s = path_for_unity_cli(project);
        let loop_name = csharp_string_literal(loop_object);
        match self {
            Self::RefreshDetect
            | Self::RunFullLoop
            | Self::ScaffoldMiniGame
            | Self::ScaffoldRpg
            | Self::ScaffoldMmo
            | Self::ScaffoldRoguelike
            | Self::EnableNpcAi
            | Self::HubLogs => {
                ("重新检测 CLI".into(), vec!["--help".into()], LoopPhase::Observe)
            }
            Self::ListEditors => (
                "列出已安装编辑器".into(),
                vec!["editors".into(), "--format".into(), "json".into()],
                LoopPhase::Observe,
            ),
            Self::ListPipeline => (
                "刷新 Pipeline 列表".into(),
                vec!["pipeline".into(), "list".into()],
                LoopPhase::Observe,
            ),
            Self::InstallPipeline => (
                "安装 com.unity.pipeline".into(),
                vec!["pipeline".into(), "install".into()],
                LoopPhase::Observe,
            ),
            Self::ListCommands | Self::ProbeEditor => {
                let title = if matches!(self, Self::ProbeEditor) {
                    "探测编辑器连接"
                } else {
                    "发现已注册命令"
                };
                (
                    title.into(),
                    vec!["command".into(), format!("--project-path={project_s}")],
                    LoopPhase::Observe,
                )
            }
            Self::Eval => eval_cli_args("Eval C# 表达式", &project_s, eval_input, LoopPhase::Act),
            Self::ObserveCollider => eval_cli_args(
                "观察碰撞体",
                &project_s,
                &format!(
                    "var go = GameObject.Find(\"{loop_name}\"); var c = go != null ? go.GetComponent<Collider>() : null; return c != null && c.enabled;"
                ),
                LoopPhase::Observe,
            ),
            Self::FixCollider => eval_cli_args(
                "修复碰撞体",
                &project_s,
                &format!(
                    "var go = GameObject.Find(\"{loop_name}\"); var c = go != null ? go.GetComponent<Collider>() : null; if (c != null) c.enabled = true; return c != null && c.enabled;"
                ),
                LoopPhase::Act,
            ),
            Self::SelectLoopObject => eval_cli_args(
                "选中闭环对象",
                &project_s,
                &format!(
                    "var go = GameObject.Find(\"{loop_name}\"); UnityEditor.Selection.activeGameObject = go; return go != null ? go.name : \"missing:{loop_name}\";"
                ),
                LoopPhase::Observe,
            ),
            Self::EnterPlayMode => eval_cli_args(
                "进入 Play Mode",
                &project_s,
                "UnityEditor.EditorApplication.isPlaying = true; return UnityEditor.EditorApplication.isPlaying;",
                LoopPhase::Verify,
            ),
            Self::ExitPlayMode => eval_cli_args(
                "退出 Play Mode",
                &project_s,
                "UnityEditor.EditorApplication.isPlaying = false; return UnityEditor.EditorApplication.isPlaying;",
                LoopPhase::Verify,
            ),
            Self::SaveScene => {
                eval_cli_args("保存场景", &project_s, EVAL_SAVE_SCENE, LoopPhase::Act)
            }
            Self::RefreshAssets => {
                eval_cli_args("刷新资源", &project_s, EVAL_REFRESH_ASSETS, LoopPhase::Act)
            }
            Self::RequestScriptReload => {
                eval_cli_args("重编译脚本", &project_s, EVAL_SCRIPT_RELOAD, LoopPhase::Act)
            }
            Self::ClearConsole => {
                eval_cli_args("清控制台", &project_s, EVAL_CLEAR_CONSOLE, LoopPhase::Act)
            }
            Self::PausePlayMode => {
                eval_cli_args("暂停/继续 Play", &project_s, EVAL_PAUSE_PLAY, LoopPhase::Act)
            }
            Self::StepPlayMode => {
                eval_cli_args("单帧步进", &project_s, EVAL_STEP_PLAY, LoopPhase::Act)
            }
            Self::UndoLast => eval_cli_args("撤销", &project_s, EVAL_UNDO, LoopPhase::Act),
            Self::RedoLast => eval_cli_args("重做", &project_s, EVAL_REDO, LoopPhase::Act),
            Self::FrameSelection => {
                eval_cli_args("框选聚焦", &project_s, EVAL_FRAME_SELECTION, LoopPhase::Act)
            }
            Self::FocusGameView => {
                eval_cli_args("切到 Game", &project_s, EVAL_FOCUS_GAME, LoopPhase::Act)
            }
            Self::FocusSceneView => {
                eval_cli_args("切到 Scene", &project_s, EVAL_FOCUS_SCENE, LoopPhase::Act)
            }
            Self::DuplicateSelection => eval_cli_args(
                "复制选中",
                &project_s,
                EVAL_DUPLICATE_SELECTION,
                LoopPhase::Act,
            ),
            Self::DeleteSelection => {
                eval_cli_args("删除选中", &project_s, EVAL_DELETE_SELECTION, LoopPhase::Act)
            }
            Self::ListScenes => {
                eval_cli_args("列构建场景", &project_s, EVAL_LIST_SCENES, LoopPhase::Observe)
            }
            Self::NewScene => eval_cli_args("新建场景", &project_s, EVAL_NEW_SCENE, LoopPhase::Act),
            Self::LoadFirstScene => {
                eval_cli_args("加载首场景", &project_s, EVAL_LOAD_FIRST_SCENE, LoopPhase::Act)
            }
            Self::HierarchyRoots => {
                eval_cli_args("场景根物体", &project_s, EVAL_HIERARCHY_ROOTS, LoopPhase::Observe)
            }
            Self::ActiveSceneInfo => {
                eval_cli_args("当前场景", &project_s, EVAL_ACTIVE_SCENE, LoopPhase::Observe)
            }
            Self::CreatePlane => {
                eval_cli_args("创建平面", &project_s, EVAL_CREATE_PLANE, LoopPhase::Act)
            }
            Self::CreateDirectionalLight => {
                eval_cli_args("创建平行光", &project_s, EVAL_CREATE_LIGHT, LoopPhase::Act)
            }
            Self::SetupSkyDay => {
                eval_cli_args("白天天空", &project_s, EVAL_SETUP_SKY_DAY, LoopPhase::Act)
            }
            Self::SetupSkySunset => {
                eval_cli_args("晚霞天空", &project_s, EVAL_SETUP_SKY_SUNSET, LoopPhase::Act)
            }
            Self::SetupSkyNight => {
                eval_cli_args("夜空", &project_s, EVAL_SETUP_SKY_NIGHT, LoopPhase::Act)
            }
            Self::CreateGround => {
                eval_cli_args("创建地面", &project_s, EVAL_CREATE_GROUND, LoopPhase::Act)
            }
            Self::SetupMainCamera => {
                eval_cli_args("设置主相机", &project_s, EVAL_SETUP_MAIN_CAMERA, LoopPhase::Act)
            }
            Self::CreatePlayerCapsule => {
                eval_cli_args("创建玩家", &project_s, EVAL_CREATE_PLAYER, LoopPhase::Act)
            }
            Self::CreateNpc => {
                eval_cli_args("创建 NPC", &project_s, EVAL_CREATE_NPC, LoopPhase::Act)
            }
            Self::CreateNpcVendor => {
                eval_cli_args("创建商人 NPC", &project_s, EVAL_CREATE_NPC_VENDOR, LoopPhase::Act)
            }
            Self::CreateNpcQuest => {
                eval_cli_args("创建任务 NPC", &project_s, EVAL_CREATE_NPC_QUEST, LoopPhase::Act)
            }
            Self::CreateSpawnPoint => {
                eval_cli_args("创建出生点", &project_s, EVAL_CREATE_SPAWN_POINT, LoopPhase::Act)
            }
            Self::CreatePortalZone => {
                eval_cli_args("创建传送门", &project_s, EVAL_CREATE_PORTAL_ZONE, LoopPhase::Act)
            }
            Self::CreateEnemySpawn => {
                eval_cli_args("创建敌人点", &project_s, EVAL_CREATE_ENEMY_SPAWN, LoopPhase::Act)
            }
            Self::InstallNpcAi => {
                let code = crate::npc_ai::eval_install_npc_ai_scripts();
                eval_cli_args("安装 NPC AI 脚本", &project_s, &code, LoopPhase::Act)
            }
            Self::AttachNpcAi => {
                eval_cli_args(
                    "挂载 NPC AI",
                    &project_s,
                    crate::npc_ai::EVAL_ATTACH_NPC_AI,
                    LoopPhase::Act,
                )
            }
            Self::LayoutRpg => {
                eval_cli_args("RPG 布局", &project_s, EVAL_LAYOUT_RPG, LoopPhase::Act)
            }
            Self::LayoutMmo => {
                eval_cli_args("MMO 布局", &project_s, EVAL_LAYOUT_MMO, LoopPhase::Act)
            }
            Self::LayoutRoguelike => {
                eval_cli_args("肉鸽布局", &project_s, EVAL_LAYOUT_ROGUELIKE, LoopPhase::Act)
            }
            Self::SaveNamedScene => {
                let path = if save_scene_path.trim().is_empty() {
                    GameGenre::Playground.scene_path()
                } else {
                    save_scene_path
                };
                let code = eval_save_named_scene(path);
                eval_cli_args("保存雏形场景", &project_s, &code, LoopPhase::Act)
            }
            Self::SaveAssets => {
                eval_cli_args("保存资源", &project_s, EVAL_SAVE_ASSETS, LoopPhase::Act)
            }
            Self::FindAssets => {
                let filter = sanitize_asset_filter(eval_input);
                let code = format!(
                    "var guids = UnityEditor.AssetDatabase.FindAssets(\"{filter}\"); int n = guids == null ? 0 : System.Math.Min(guids.Length, 30); var sb = new System.Text.StringBuilder(); sb.Append(\"count=\").Append(guids == null ? 0 : guids.Length); for (int i = 0; i < n; i++) {{ sb.Append('\\n').Append(UnityEditor.AssetDatabase.GUIDToAssetPath(guids[i])); }} return sb.ToString();"
                );
                eval_cli_args("搜索资源", &project_s, &code, LoopPhase::Observe)
            }
            Self::ConsoleErrors => {
                eval_cli_args("控制台错误", &project_s, EVAL_CONSOLE_ERRORS, LoopPhase::Observe)
            }
            Self::FindMissingScripts => {
                eval_cli_args("缺失脚本", &project_s, EVAL_MISSING_SCRIPTS, LoopPhase::Observe)
            }
            Self::ListPackages => {
                eval_cli_args("列出包", &project_s, EVAL_LIST_PACKAGES, LoopPhase::Observe)
            }
            Self::AddPackage => {
                let pkg = sanitize_package_id(eval_input);
                let code = format!(
                    "var req = UnityEditor.PackageManager.Client.Add(\"{pkg}\"); while (!req.IsCompleted) System.Threading.Thread.Sleep(50); if (req.Status == UnityEditor.PackageManager.StatusCode.Success) return req.Result.packageId; return req.Error != null ? req.Error.message : \"Add failed\";"
                );
                eval_cli_args("安装包", &project_s, &code, LoopPhase::Act)
            }
            Self::BuildWindowsPlayer => {
                eval_cli_args("构建 Win64", &project_s, EVAL_BUILD_WIN64, LoopPhase::Verify)
            }
            Self::EditorStatus => (
                "编辑器状态".into(),
                vec!["status".into(), "--format".into(), "json".into()],
                LoopPhase::Observe,
            ),
            Self::ListProjects => (
                "Hub 工程列表".into(),
                vec![
                    "projects".into(),
                    "list".into(),
                    "--json".into(),
                    "--all".into(),
                ],
                LoopPhase::Observe,
            ),
            Self::OpenProject => (
                "打开工程".into(),
                vec!["open".into(), project_s.clone()],
                LoopPhase::Act,
            ),
            Self::RequireEditor => (
                "补齐所需编辑器".into(),
                vec!["projects".into(), "require".into(), project_s.clone()],
                LoopPhase::Observe,
            ),
            Self::UpgradePipeline => (
                "升级 Pipeline".into(),
                vec!["pipeline".into(), "upgrade".into()],
                LoopPhase::Observe,
            ),
            Self::ProjectInfo => (
                "工程信息".into(),
                vec![
                    "projects".into(),
                    "info".into(),
                    project_s.clone(),
                    "--json".into(),
                ],
                LoopPhase::Observe,
            ),
            Self::RegisterProject => (
                "注册到 Hub".into(),
                vec![
                    "projects".into(),
                    "add".into(),
                    project_s.clone(),
                    "--json".into(),
                ],
                LoopPhase::Act,
            ),
            Self::PinProject => (
                "收藏工程".into(),
                vec![
                    "projects".into(),
                    "pin".into(),
                    project_s.clone(),
                    "--json".into(),
                ],
                LoopPhase::Act,
            ),
            Self::ListLtsReleases => (
                "LTS 版本".into(),
                vec![
                    "editors".into(),
                    "--releases".into(),
                    "--format".into(),
                    "json".into(),
                ],
                LoopPhase::Observe,
            ),
            Self::CacheInfo => (
                "下载缓存".into(),
                vec!["cache".into(), "info".into()],
                LoopPhase::Observe,
            ),
            Self::RunEditModeTests => {
                let out = path_for_unity_cli(
                    &crate::usage::usage_dir().join("unity-edit-results.xml"),
                );
                (
                    "EditMode 测试".into(),
                    vec![
                        "test".into(),
                        project_s.clone(),
                        "--mode".into(),
                        "EditMode".into(),
                        "--output".into(),
                        out,
                    ],
                    LoopPhase::Verify,
                )
            }
            Self::RunPlayModeTests => {
                let out = path_for_unity_cli(
                    &crate::usage::usage_dir().join("unity-play-results.xml"),
                );
                (
                    "PlayMode 测试".into(),
                    vec![
                        "test".into(),
                        project_s.clone(),
                        "--mode".into(),
                        "PlayMode".into(),
                        "--output".into(),
                        out,
                    ],
                    LoopPhase::Verify,
                )
            }
            Self::Doctor => (
                "环境诊断".into(),
                vec!["doctor".into(), "--format".into(), "json".into()],
                LoopPhase::Observe,
            ),
            Self::EnvInfo => (
                "Hub 环境".into(),
                vec!["env".into(), "--format".into(), "json".into()],
                LoopPhase::Observe,
            ),
            Self::LicenseInfo => (
                "许可信息".into(),
                vec!["license".into(), "--format".into(), "json".into()],
                LoopPhase::Observe,
            ),
        }
    }
}

fn eval_cli_args(
    title: &str,
    project_s: &str,
    code: &str,
    phase: LoopPhase,
) -> (String, Vec<String>, LoopPhase) {
    (
        title.into(),
        vec![
            "--format".into(),
            "json".into(),
            "command".into(),
            format!("--project-path={project_s}"),
            "eval".into(),
            "--".into(),
            "--code".into(),
            code.to_string(),
        ],
        phase,
    )
}

fn csharp_string_literal(raw: &str) -> String {
    let trimmed = raw.trim();
    let safe = if trimmed.is_empty() { "Ground" } else { trimmed };
    safe.chars()
        .filter(|c| *c != '\0' && *c != '\n' && *c != '\r')
        .take(64)
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn sanitize_asset_filter(raw: &str) -> String {
    let trimmed = raw.trim();
    let candidate = if trimmed.is_empty()
        || trimmed.contains(';')
        || trimmed.to_ascii_lowercase().contains("return ")
    {
        "t:Prefab"
    } else {
        trimmed
    };
    candidate
        .chars()
        .filter(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | ' ' | '/' | '*')
        })
        .take(80)
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn sanitize_package_id(raw: &str) -> String {
    let trimmed = raw.trim();
    let candidate = if trimmed.is_empty()
        || trimmed.contains(';')
        || trimmed.to_ascii_lowercase().contains("return ")
    {
        "com.unity.ugui"
    } else {
        trimmed
    };
    candidate
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
        .take(120)
        .collect()
}

fn hub_logs_dir_display() -> String {
    if cfg!(windows) {
        std::env::var("USERPROFILE")
            .map(|home| format!(r"{home}\AppData\Roaming\UnityHub\logs"))
            .unwrap_or_else(|_| r"%UserProfile%\AppData\Roaming\UnityHub\logs".into())
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .map(|home| format!("{home}/Library/Application Support/UnityHub/logs"))
            .unwrap_or_else(|_| "~/Library/Application Support/UnityHub/logs".into())
    } else {
        std::env::var("HOME")
            .map(|home| format!("{home}/.config/UnityHub/logs"))
            .unwrap_or_else(|_| "~/.config/UnityHub/logs".into())
    }
}

fn path_for_unity_cli(path: &PathBuf) -> String {
    let raw = path.display().to_string();
    #[cfg(windows)]
    {
        if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    raw
}

fn display_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        return PathBuf::from(path_for_unity_cli(&path));
    }
    #[cfg(not(windows))]
    path
}

struct DetectResult {
    path: Option<PathBuf>,
    version: String,
    error: Option<String>,
}

struct RunResult {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn detect_cli(cancel: Option<&Arc<AtomicBool>>) -> DetectResult {
    let candidates = candidate_bins();
    // Probe every candidate path concurrently instead of serially (each probe
    // can take up to 8s); join in priority order so the earliest-listed
    // candidate still wins ties, but total wall time is ~max, not ~sum.
    let handles: Vec<_> = candidates
        .into_iter()
        .map(|path| {
            let cancel = cancel.cloned();
            thread::spawn(move || {
                let result = run_unity_timeout(
                    &path,
                    &["--help".into()],
                    Duration::from_secs(8),
                    None,
                    cancel.as_ref(),
                );
                (path, result)
            })
        })
        .collect();

    let mut first_error = None;
    for handle in handles {
        let Ok((path, result)) = handle.join() else {
            continue;
        };
        if result.ok
            || result.stdout.contains("Usage")
            || result.stdout.to_lowercase().contains("unity")
        {
            let version = first_nonempty_line(&result.stdout)
                .or_else(|| first_nonempty_line(&result.stderr))
                .unwrap_or_else(|| "unity CLI".into());
            return DetectResult {
                path: Some(path),
                version,
                error: None,
            };
        }
        if first_error.is_none() && !result.stderr.is_empty() {
            first_error = Some(result.stderr);
        }
    }
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return DetectResult {
            path: None,
            version: String::new(),
            error: first_error,
        };
    }

    match crate::process::command("unity").arg("--help").output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success()
                || stdout.contains("Usage")
                || stdout.to_lowercase().contains("unity")
            {
                let version = first_nonempty_line(&stdout)
                    .or_else(|| first_nonempty_line(&stderr))
                    .unwrap_or_else(|| "unity (PATH)".into());
                return DetectResult {
                    path: which_unity(),
                    version,
                    error: None,
                };
            }
            DetectResult {
                path: None,
                version: String::new(),
                error: Some(truncate_one_line(&stderr, 200)),
            }
        }
        Err(err) => DetectResult {
            path: None,
            version: String::new(),
            error: Some(format!("无法启动 unity: {err}")),
        },
    }
}

fn candidate_bins() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("UNITY_CLI") {
        out.push(PathBuf::from(p));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local);
        out.push(base.join("Unity").join("bin").join("unity.exe"));
        out.push(base.join("Unity").join("cli").join("unity.exe"));
        out.push(
            base.join("Programs")
                .join("Unity")
                .join("cli")
                .join("unity.exe"),
        );
        out.push(
            base.join("Programs")
                .join("Unity")
                .join("bin")
                .join("unity.exe"),
        );
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let base = PathBuf::from(home);
        out.push(base.join(".unity").join("bin").join("unity.exe"));
        out.push(base.join(".unity").join("cli").join("unity.exe"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let base = PathBuf::from(home);
        out.push(base.join(".unity").join("bin").join("unity"));
        out.push(base.join(".local").join("bin").join("unity"));
    }
    out.into_iter().filter(|p| p.exists()).collect()
}

fn which_unity() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let output = crate::process::command("where.exe").arg("unity").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().next().map(|l| PathBuf::from(l.trim()))
    }
    #[cfg(not(windows))]
    {
        let output = crate::process::command("sh")
            .args(["-c", "command -v unity"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().next()?.trim();
        if line.is_empty() {
            None
        } else {
            Some(PathBuf::from(line))
        }
    }
}

fn run_unity_timeout(
    bin: &PathBuf,
    args: &[String],
    timeout: Duration,
    cwd: Option<&PathBuf>,
    cancel: Option<&Arc<AtomicBool>>,
) -> RunResult {
    let mut cmd = crate::process::command(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CI", "1")
        .env("UNITY_CLI_NONINTERACTIVE", "1");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return RunResult {
                ok: false,
                stdout: String::new(),
                stderr: format!("spawn failed: {err}"),
            };
        }
    };

    let started = Instant::now();
    let status = loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            return RunResult {
                ok: false,
                stdout: String::new(),
                stderr: "cancelled".into(),
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return RunResult {
                    ok: false,
                    stdout: String::new(),
                    stderr: format!("timeout after {}s", timeout.as_secs()),
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(err) => {
                return RunResult {
                    ok: false,
                    stdout: String::new(),
                    stderr: format!("wait failed: {err}"),
                };
            }
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    RunResult {
        ok: status.success(),
        stdout,
        stderr,
    }
}

fn build_install_command() -> Command {
    if cfg!(windows) {
        let mut cmd = crate::process::command("powershell");
        cmd.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            UnityState::install_hint_windows(),
        ]);
        cmd
    } else {
        let mut cmd = crate::process::command("bash");
        cmd.args(["-c", UnityState::install_hint_unix()]);
        cmd
    }
}

fn run_cli_install(
    id: u64,
    tx: &mpsc::Sender<(u64, UnityWorkerMsg)>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut cmd = build_install_command();
    run_streaming_tagged(&mut cmd, id, tx, cancel)
}

fn run_streaming_tagged(
    cmd: &mut Command,
    id: u64,
    tx: &mpsc::Sender<(u64, UnityWorkerMsg)>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动安装脚本：{e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let out_handle = thread::spawn(move || {
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().flatten() {
                let _ = tx_out.send((id, UnityWorkerMsg::InstallProgress { line }));
            }
        }
    });
    let tx_err = tx.clone();
    let err_handle = thread::spawn(move || {
        let mut last = String::new();
        if let Some(err) = stderr {
            for line in BufReader::new(err).lines().flatten() {
                last = line.clone();
                let _ = tx_err.send((id, UnityWorkerMsg::InstallProgress { line }));
            }
        }
        last
    });

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err("已取消安装".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => thread::sleep(Duration::from_millis(80)),
            Err(e) => return Err(format!("等待安装进程失败：{e}")),
        }
    };
    let _ = out_handle.join();
    let err_tail = err_handle.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "安装脚本退出码 {}{}",
            status.code().unwrap_or(-1),
            if err_tail.is_empty() {
                String::new()
            } else {
                format!("：{err_tail}")
            }
        ))
    }
}

fn push_install_log(tail: &mut Vec<String>, line: String) {
    let line = line.trim_end().to_string();
    if line.is_empty() {
        return;
    }
    tail.push(line);
    if tail.len() > INSTALL_LOG_TAIL_MAX {
        let drain = tail.len() - INSTALL_LOG_TAIL_MAX;
        tail.drain(0..drain);
    }
}

fn format_command(bin: &PathBuf, args: &[String]) -> String {
    let mut parts = vec![bin.display().to_string()];
    for a in args {
        if a.contains(' ') {
            parts.push(format!("\"{a}\""));
        } else {
            parts.push(a.clone());
        }
    }
    parts.join(" ")
}

fn summarize_editors_json(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return truncate_one_line(raw, 120);
    };
    if let Some(arr) = value.as_array() {
        if arr.is_empty() {
            return "0 个已安装编辑器".into();
        }
        let versions: Vec<String> = arr
            .iter()
            .filter_map(|v| {
                v.get("version")
                    .or_else(|| v.get("Version"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
            .take(4)
            .collect();
        if versions.is_empty() {
            return format!("{} 个编辑器", arr.len());
        }
        return format!("{} 个：{}", arr.len(), versions.join(", "));
    }
    truncate_one_line(raw, 120)
}

fn summarize_status_json(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return truncate_one_line(raw, 120);
    };
    let data = value.get("data").unwrap_or(&value);
    let count = data
        .get("count")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            data.get("instances")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(0);
    if count == 0 {
        return "无已连接编辑器（需打开工程并安装 Pipeline）".into();
    }
    let mut parts = Vec::new();
    if let Some(arr) = data.get("instances").and_then(|v| v.as_array()) {
        for inst in arr.iter().take(3) {
            let project = inst
                .get("project")
                .or_else(|| inst.get("projectPath"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let version = inst
                .get("version")
                .or_else(|| inst.get("editorVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let state = inst
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("connected");
            parts.push(format!(
                "{} · {} · {}",
                truncate_one_line(project, 40),
                version,
                state
            ));
        }
    }
    if parts.is_empty() {
        format!("{count} 个已连接编辑器")
    } else {
        format!("{count} 个：{}", parts.join("；"))
    }
}

fn status_has_instances(raw: &str) -> Option<bool> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let data = value.get("data").unwrap_or(&value);
    let count = data.get("count").and_then(|v| v.as_u64()).or_else(|| {
        data.get("instances")
            .and_then(|v| v.as_array())
            .map(|a| a.len() as u64)
    })?;
    Some(count > 0)
}

fn summarize_projects_json(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return truncate_one_line(raw, 160);
    };
    let arr = value
        .as_array()
        .or_else(|| value.get("projects").and_then(|v| v.as_array()))
        .or_else(|| value.get("data").and_then(|v| v.as_array()))
        .or_else(|| {
            value
                .get("data")
                .and_then(|d| d.get("projects"))
                .and_then(|v| v.as_array())
        });
    let Some(arr) = arr else {
        return truncate_one_line(raw, 160);
    };
    if arr.is_empty() {
        return "Hub 中无已注册工程".into();
    }
    let mut lines = vec![format!("Hub 工程 {} 个：", arr.len())];
    for p in arr.iter().take(8) {
        let name = p
            .get("title")
            .or_else(|| p.get("name"))
            .or_else(|| p.get("Name"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let path = p
            .get("path")
            .or_else(|| p.get("Path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ver = p
            .get("version")
            .or_else(|| p.get("editorVersion"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if path.is_empty() {
            lines.push(format!("· {name} {ver}"));
        } else {
            lines.push(format!(
                "· {name} ({ver}) — 可用聊天设工程：{}",
                truncate_one_line(path, 60)
            ));
        }
    }
    if arr.len() > 8 {
        lines.push(format!("…另有 {} 个", arr.len() - 8));
    }
    lines.join("\n")
}

fn summarize_test_output(action: UnityAction, stdout: &str, stderr: &str) -> String {
    let mode = match action {
        UnityAction::RunEditModeTests => "EditMode",
        UnityAction::RunPlayModeTests => "PlayMode",
        _ => "Test",
    };
    let blob = if !stdout.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let lower = blob.to_lowercase();
    let out_hint = if matches!(action, UnityAction::RunEditModeTests) {
        "unity-edit-results.xml"
    } else {
        "unity-play-results.xml"
    };
    if lower.contains("fail") || lower.contains("error") {
        format!("{mode} 测试有失败/错误 · 详见 ~/.bony-build/{out_hint}")
    } else if blob.trim().is_empty() {
        format!("{mode} 测试已结束 · 结果 ~/.bony-build/{out_hint}")
    } else {
        format!(
            "{mode} · {} · ~/.bony-build/{out_hint}",
            truncate_one_line(blob, 80)
        )
    }
}

fn summarize_releases_json(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return truncate_one_line(raw, 160);
    };
    let arr = value
        .as_array()
        .or_else(|| value.get("releases").and_then(|v| v.as_array()))
        .or_else(|| value.get("data").and_then(|v| v.as_array()))
        .or_else(|| {
            value
                .get("data")
                .and_then(|d| d.get("releases"))
                .and_then(|v| v.as_array())
        });
    let Some(arr) = arr else {
        return truncate_one_line(raw, 160);
    };
    if arr.is_empty() {
        return "无可用 LTS 版本".into();
    }
    let versions: Vec<String> = arr
        .iter()
        .filter_map(|v| {
            v.get("version")
                .or_else(|| v.get("Version"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .take(5)
        .collect();
    if versions.is_empty() {
        format!("{} 个 LTS 版本", arr.len())
    } else {
        format!("LTS {} 个：{}", arr.len(), versions.join(", "))
    }
}

fn summarize_pipeline_list(raw: &str) -> String {
    let lower = raw.to_lowercase();
    // The CLI table may wrap between any header columns. Detect the two
    // relevant labels independently so `Server\nReachable` is still valid.
    if lower.contains("server") && lower.contains("reachable") {
        let version = raw
            .split_whitespace()
            .find(|field| {
                field.chars().next().is_some_and(|c| c.is_ascii_digit()) && field.contains('.')
            })
            .unwrap_or("未知版本");
        return match pipeline_server_reachable(raw) {
            Some(true) => format!("Pipeline {version} · 编辑器服务可达"),
            Some(false) => format!("Pipeline {version} · 包已登记，编辑器服务尚未启动"),
            None => format!("Pipeline {version} · 尚未识别编辑器服务状态"),
        };
    }
    if lower.contains("server reachable") {
        let data = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .skip(1)
            .find(|line| line.split_whitespace().count() > 2);
        if let Some(line) = data {
            let fields: Vec<_> = line.split_whitespace().collect();
            let version = fields
                .iter()
                .find(|field| {
                    field.chars().next().is_some_and(|c| c.is_ascii_digit()) && field.contains('.')
                })
                .copied()
                .unwrap_or("未知版本");
            let reachable = pipeline_server_reachable(raw).unwrap_or(false);
            return if reachable {
                format!("Pipeline {version} · 编辑器服务可达")
            } else {
                format!("Pipeline {version} · 包已登记，编辑器服务尚未启动")
            };
        }
        "Pipeline 已登记 · 尚未发现编辑器服务".into()
    } else if lower.contains("installed") {
        "Pipeline: Installed".into()
    } else if raw.lines().filter(|l| !l.trim().is_empty()).count() == 0 {
        "无 Pipeline 项目".into()
    } else {
        truncate_one_line(raw, 160)
    }
}

/// Finds the reachability boolean using the server port as a stable anchor.
/// This survives missing PID values and terminal line wrapping.
fn pipeline_server_reachable(raw: &str) -> Option<bool> {
    let fields: Vec<_> = raw.split_whitespace().collect();
    let version_index = fields.iter().position(|field| {
        field.chars().next().is_some_and(|c| c.is_ascii_digit()) && field.contains('.')
    })?;
    fields[version_index + 1..].windows(2).find_map(|pair| {
        pair[0].parse::<u16>().ok()?;
        match pair[1].to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    })
}

fn infer_pipeline_status(raw: &str, ok: bool) -> PipelineStatus {
    if !ok {
        return PipelineStatus::Error;
    }
    let lower = raw.to_lowercase();
    if lower.contains("not installed") || lower.contains("notinstalled") {
        PipelineStatus::NotInstalled
    } else if lower.contains("installed")
        || lower.contains("com.unity.pipeline")
        || lower.contains("pipeline: installed")
    {
        PipelineStatus::Installed
    } else if raw.trim().is_empty() {
        PipelineStatus::NotInstalled
    } else {
        // Non-empty list output usually means at least one project.
        PipelineStatus::Installed
    }
}

fn summarize_eval_output(raw: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(result) = value
            .pointer("/data/result")
            .or_else(|| value.get("result"))
        {
            return truncate_one_line(&result.to_string(), 160);
        }
        if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
            return value
                .pointer("/errors/0/message")
                .and_then(|v| v.as_str())
                .map(|v| truncate_one_line(v, 160))
                .unwrap_or_else(|| "Unity Eval 失败".into());
        }
    }
    raw.lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line
                    .to_ascii_lowercase()
                    .contains("command success result parameters")
        })
        .map(|line| truncate_one_line(line, 160))
        .unwrap_or_else(|| "Unity Eval 未返回结果".into())
}

fn eval_output_succeeded(raw: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    if value.pointer("/data/success").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    value.get("success").and_then(|v| v.as_bool()) == Some(true)
        || value.pointer("/data/success").and_then(|v| v.as_bool()) == Some(true)
}

fn infer_editor_link(raw: &str, ok: bool) -> EditorLinkStatus {
    if !ok {
        return EditorLinkStatus::Disconnected;
    }
    let lower = raw.to_lowercase();
    if lower.contains("no editor")
        || lower.contains("not connected")
        || lower.contains("could not")
        || lower.contains("failed")
    {
        EditorLinkStatus::Disconnected
    } else if raw.trim().is_empty() {
        EditorLinkStatus::Disconnected
    } else {
        EditorLinkStatus::Connected
    }
}

/// Public helpers for the desktop UI.
pub fn is_unity_project_root(path: &PathBuf) -> bool {
    looks_like_unity_project(path)
}

pub fn find_unity_project_root(path: &PathBuf) -> Option<PathBuf> {
    resolve_unity_project_root(path)
}

fn looks_like_unity_project(path: &PathBuf) -> bool {
    path.join("Assets").is_dir()
        && (path.join("ProjectSettings").is_dir()
            || path.join("Packages").join("manifest.json").is_file())
}

fn pipeline_declared(project: &PathBuf) -> bool {
    std::fs::read_to_string(project.join("Packages").join("manifest.json"))
        .is_ok_and(|text| text.contains("\"com.unity.pipeline\""))
}

fn pipeline_loaded_by_editor(project: &PathBuf) -> bool {
    let locked = std::fs::read_to_string(project.join("Packages").join("packages-lock.json"))
        .is_ok_and(|text| text.contains("\"com.unity.pipeline\""));
    let cached = std::fs::read_dir(project.join("Library").join("PackageCache"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("com.unity.pipeline@")
        });
    locked && cached
}

/// Walk up from `path` until we find a Unity project root (Assets + ProjectSettings/Packages).
/// Handles cases where cwd is inside `Assets/...` or another subfolder.
fn resolve_unity_project_root(path: &PathBuf) -> Option<PathBuf> {
    let mut cur = path.canonicalize().unwrap_or_else(|_| path.clone());
    for _ in 0..12 {
        if looks_like_unity_project(&cur) {
            return Some(display_path(cur));
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn unity_project_pref_path() -> PathBuf {
    crate::usage::usage_dir().join("unity_project.json")
}

fn load_unity_project_pref() -> Option<PathBuf> {
    let text = std::fs::read_to_string(unity_project_pref_path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = value.get("path")?.as_str()?;
    let p = PathBuf::from(path);
    p.is_dir().then_some(p)
}

fn save_unity_project_pref(path: &PathBuf) {
    let dir = crate::usage::usage_dir();
    let _ = std::fs::create_dir_all(&dir);
    let body = serde_json::json!({ "path": path });
    if let Ok(text) = serde_json::to_string_pretty(&body) {
        let _ = std::fs::write(unity_project_pref_path(), text);
    }
}

fn demo_eval_result(expr: &str, scene: &SceneSnapshot) -> String {
    let lower = expr.to_lowercase();
    if lower.contains("isplaying") {
        return if scene.is_playing {
            "true".into()
        } else {
            "false".into()
        };
    }
    if lower.contains("collider") || lower.contains("enabled") {
        return if scene.ground_collider_enabled {
            "true".into()
        } else {
            "false".into()
        };
    }
    if lower.contains("version") {
        return "\"6000.2.10f1\"".into();
    }
    if lower.contains("datapath") {
        return "\"C:/Projects/DemoGame/Assets\"".into();
    }
    "null".into()
}

fn merge_streams(stdout: &str, stderr: &str) -> String {
    if stderr.is_empty() {
        stdout.to_string()
    } else if stdout.is_empty() {
        stderr.to_string()
    } else {
        format!("{stdout}\n--- stderr ---\n{stderr}")
    }
}

fn truncate_one_line(s: &str, max: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn first_nonempty_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn format_relative(at_unix: u64) -> String {
    let now = now_unix();
    let secs = now.saturating_sub(at_unix);
    if secs < 5 {
        "刚刚".into()
    } else if secs < 60 {
        format!("{secs} 秒前")
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86400 {
        format!("{} 小时前", secs / 3600)
    } else {
        format!("{} 天前", secs / 86400)
    }
}

const DEMO_EDITORS_JSON: &str = r#"[
  {"version":"6000.2.10f1","modules":["android","ios","webgl"]},
  {"version":"6000.0.28f1","modules":["android"]}
]"#;

const DEMO_STATUS_JSON: &str = r#"{
  "success": true,
  "command": "status",
  "data": {
    "count": 1,
    "instances": [
      {
        "project": "C:/Demo/UnityProject",
        "version": "6000.2.10f1",
        "state": "ready",
        "port": 39000
      }
    ]
  },
  "errors": [],
  "warnings": []
}"#;

const DEMO_PROJECTS_JSON: &str = r#"[
  {"title":"Demo Game","path":"C:/Demo/UnityProject","version":"6000.2.10f1"},
  {"title":"Sandbox","path":"C:/Demo/Sandbox","version":"6000.0.28f1"}
]"#;

const DEMO_RELEASES_JSON: &str = r#"[
  {"version":"6000.0.28f1","stream":"lts"},
  {"version":"2022.3.50f1","stream":"lts"},
  {"version":"2021.3.45f1","stream":"lts"}
]"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_job_messages_are_dropped() {
        let mut state = UnityState::default();
        // Simulate a job (id 1) that is still in flight...
        state.job_seq = 1;
        let (tx, rx) = mpsc::channel::<(u64, UnityWorkerMsg)>();
        state.pending_rx = Some(rx);
        // ...but gets superseded by a second job before it replies.
        state.job_seq = 2;
        tx.send((
            1,
            UnityWorkerMsg::Detected {
                path: Some(PathBuf::from("stale-path")),
                version: "stale".into(),
                error: None,
            },
        ))
        .unwrap();

        let changed = state.drain_worker();

        assert!(!changed, "a reply from a superseded job must be ignored");
        assert_eq!(state.cli_path, None);
        assert_ne!(state.status, CliStatus::Ready);
    }

    #[test]
    fn current_job_messages_are_applied() {
        let mut state = UnityState::default();
        state.job_seq = 5;
        let (tx, rx) = mpsc::channel::<(u64, UnityWorkerMsg)>();
        state.pending_rx = Some(rx);
        state.busy = true;
        tx.send((
            5,
            UnityWorkerMsg::Detected {
                path: Some(PathBuf::from("real-path")),
                version: "1.2.3".into(),
                error: None,
            },
        ))
        .unwrap();

        let changed = state.drain_worker();

        assert!(changed);
        assert_eq!(state.cli_path, Some(PathBuf::from("real-path")));
        assert_eq!(state.status, CliStatus::Ready);
        assert!(!state.busy);
    }

    #[test]
    fn spawn_job_cancels_previous_in_flight_job() {
        let mut state = UnityState::default();
        let (first_seen_cancel_tx, first_seen_cancel_rx) = mpsc::channel::<bool>();
        let cancel1 = state.spawn_job(move |_id, _tx, cancel| {
            let mut noticed = false;
            for _ in 0..100 {
                if cancel.load(Ordering::Relaxed) {
                    noticed = true;
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = first_seen_cancel_tx.send(noticed);
        });

        // Spawning a second job must cooperatively cancel the first.
        let _cancel2 = state.spawn_job(move |_id, _tx, _cancel| {});

        let noticed = first_seen_cancel_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("first job should observe the cancel flag");
        assert!(noticed);
        assert!(cancel1.load(Ordering::Relaxed));
    }

    #[test]
    fn run_unity_timeout_respects_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let flipper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            cancel_clone.store(true, Ordering::SeqCst);
        });

        let (bin, args): (PathBuf, Vec<String>) = if cfg!(windows) {
            (
                PathBuf::from("ping"),
                vec!["-n".into(), "10".into(), "127.0.0.1".into()],
            )
        } else {
            (PathBuf::from("sleep"), vec!["10".into()])
        };

        let started = Instant::now();
        let result = run_unity_timeout(&bin, &args, Duration::from_secs(30), None, Some(&cancel));
        flipper.join().unwrap();

        assert_eq!(result.stderr, "cancelled");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "cancellation should abort the wait loop well before the real timeout"
        );
    }

    #[test]
    fn summarize_demo_status_and_projects() {
        let status = summarize_status_json(DEMO_STATUS_JSON);
        assert!(status.contains("1"));
        assert!(status_has_instances(DEMO_STATUS_JSON) == Some(true));
        let projects = summarize_projects_json(DEMO_PROJECTS_JSON);
        assert!(projects.contains("Demo Game"));
        assert!(projects.contains("Sandbox"));
        let releases = summarize_releases_json(DEMO_RELEASES_JSON);
        assert!(releases.contains("LTS"));
        assert!(releases.contains("6000.0.28f1"));
    }

    #[test]
    fn productivity_actions_map_to_cli() {
        let project = PathBuf::from(r"C:\Unity\Project");
        let playground = GameGenre::Playground.scene_path();
        let (_, args, _) = UnityAction::SaveScene.to_cli_args("", &project, "Ground", playground);
        assert!(args.iter().any(|a| a == "eval"));
        let (_, args, _) = UnityAction::UndoLast.to_cli_args("", &project, "Ground", playground);
        assert!(args.iter().any(|a| a == "eval"));
        let (_, args, _) = UnityAction::ListScenes.to_cli_args("", &project, "Ground", playground);
        assert!(args.iter().any(|a| a == "eval"));
        let (_, args, _) = UnityAction::BuildWindowsPlayer.to_cli_args("", &project, "Floor", playground);
        assert!(args.iter().any(|a| a == "eval"));
        let (_, args, _) = UnityAction::ObserveCollider.to_cli_args("", &project, "Floor", playground);
        let code = args.last().cloned().unwrap_or_default();
        assert!(code.contains("Floor"));
        let (_, args, _) = UnityAction::EditorStatus.to_cli_args("", &project, "Ground", playground);
        assert_eq!(args[0], "status");
        let (_, args, _) = UnityAction::ListProjects.to_cli_args("", &project, "Ground", playground);
        assert_eq!(args[0], "projects");
        let (_, args, _) = UnityAction::UpgradePipeline.to_cli_args("", &project, "Ground", playground);
        assert_eq!(args[..2], ["pipeline".to_string(), "upgrade".to_string()]);
        let (_, args, _) = UnityAction::RegisterProject.to_cli_args("", &project, "Ground", playground);
        assert_eq!(args[1], "add");
        let (_, args, _) = UnityAction::ListLtsReleases.to_cli_args("", &project, "Ground", playground);
        assert_eq!(args[0], "editors");
        assert!(args.iter().any(|a| a == "--releases"));
        let (_, args, _) = UnityAction::RunEditModeTests.to_cli_args("", &project, "Ground", playground);
        assert_eq!(args[0], "test");
        assert!(args.iter().any(|a| a == "EditMode"));
    }

    #[test]
    fn full_loop_queues_three_steps() {
        let mut state = UnityState::default();
        state.status = CliStatus::Missing;
        state.demo_mode = true;
        state.run_action(UnityAction::RunFullLoop);
        assert_eq!(state.guide_queue.len(), 3);
        assert!(state.guide_label.is_some());
        assert_eq!(state.guide_kind, GuideKind::Loop);
    }

    #[test]
    fn scaffold_mini_game_queues_creation_pipeline() {
        let mut state = UnityState::default();
        state.status = CliStatus::Missing;
        state.demo_mode = true;
        state.run_action(UnityAction::ScaffoldMiniGame);
        assert_eq!(state.guide_total, 8);
        assert_eq!(state.guide_queue.len(), 8);
        assert_eq!(state.guide_kind, GuideKind::Scaffold);
        assert_eq!(state.guide_genre, Some(GameGenre::Playground));
        assert_eq!(
            state.scaffold_save_path,
            GameGenre::Playground.scene_path()
        );
        assert_eq!(state.guide_queue[0], UnityAction::NewScene);
        assert_eq!(state.guide_queue[1], UnityAction::SetupSkyDay);
        assert_eq!(state.guide_queue[2], UnityAction::CreateGround);
        assert_eq!(state.guide_queue[3], UnityAction::CreateDirectionalLight);
        assert_eq!(state.guide_queue[4], UnityAction::SetupMainCamera);
        assert_eq!(state.guide_queue[5], UnityAction::CreatePlayerCapsule);
        assert_eq!(state.guide_queue[6], UnityAction::SaveNamedScene);
        assert_eq!(state.guide_queue[7], UnityAction::EnterPlayMode);
        assert!(state.guide_label.as_deref().unwrap_or("").contains("创作"));
    }

    #[test]
    fn scaffold_rpg_queues_layout_and_save_path() {
        let mut state = UnityState::default();
        state.status = CliStatus::Missing;
        state.demo_mode = true;
        state.run_action(UnityAction::ScaffoldRpg);
        assert_eq!(state.guide_total, 8);
        assert_eq!(state.guide_genre, Some(GameGenre::Rpg));
        assert_eq!(state.scaffold_save_path, GameGenre::Rpg.scene_path());
        assert_eq!(state.guide_queue[0], UnityAction::NewScene);
        assert_eq!(state.guide_queue[1], UnityAction::SetupSkyDay);
        assert_eq!(state.guide_queue[5], UnityAction::LayoutRpg);
        assert_eq!(state.guide_queue[6], UnityAction::SaveNamedScene);
        assert_eq!(state.guide_queue[7], UnityAction::EnterPlayMode);
    }

    #[test]
    fn scaffold_mmo_queues_hub_layout() {
        let mut state = UnityState::default();
        state.status = CliStatus::Missing;
        state.demo_mode = true;
        state.run_action(UnityAction::ScaffoldMmo);
        assert_eq!(state.guide_total, 7);
        assert_eq!(state.guide_genre, Some(GameGenre::Mmo));
        assert_eq!(state.scaffold_save_path, GameGenre::Mmo.scene_path());
        assert_eq!(state.guide_queue[0], UnityAction::NewScene);
        assert_eq!(state.guide_queue[1], UnityAction::SetupSkyDay);
        assert_eq!(state.guide_queue[2], UnityAction::CreateDirectionalLight);
        assert_eq!(state.guide_queue[3], UnityAction::SetupMainCamera);
        assert_eq!(state.guide_queue[4], UnityAction::LayoutMmo);
        assert_eq!(state.guide_queue[5], UnityAction::SaveNamedScene);
        assert_eq!(state.guide_queue[6], UnityAction::EnterPlayMode);
    }

    #[test]
    fn scaffold_roguelike_uses_night_sky() {
        let mut state = UnityState::default();
        state.status = CliStatus::Missing;
        state.demo_mode = true;
        state.run_action(UnityAction::ScaffoldRoguelike);
        assert_eq!(state.guide_total, 7);
        assert_eq!(state.guide_genre, Some(GameGenre::Roguelike));
        assert_eq!(
            state.scaffold_save_path,
            GameGenre::Roguelike.scene_path()
        );
        assert_eq!(state.guide_queue[0], UnityAction::NewScene);
        assert_eq!(state.guide_queue[1], UnityAction::SetupSkyNight);
        assert_eq!(state.guide_queue[4], UnityAction::LayoutRoguelike);
        assert_eq!(state.guide_queue[5], UnityAction::SaveNamedScene);
        assert_eq!(state.guide_queue[6], UnityAction::EnterPlayMode);
    }

    #[test]
    fn genre_layout_evals_contain_markers() {
        assert!(EVAL_LAYOUT_RPG.contains("NPC_Quest"));
        assert!(EVAL_LAYOUT_RPG.contains("NPC_Vendor"));
        assert!(EVAL_LAYOUT_RPG.contains("Spawn_Town"));
        assert!(EVAL_LAYOUT_MMO.contains("Spawn_A"));
        assert!(EVAL_LAYOUT_MMO.contains("Portal_Zone"));
        assert!(EVAL_LAYOUT_MMO.contains("ChatPanel"));
        assert!(EVAL_LAYOUT_ROGUELIKE.contains("Door_North"));
        assert!(EVAL_LAYOUT_ROGUELIKE.contains("Enemy_Spawn"));
        assert!(EVAL_LAYOUT_ROGUELIKE.contains("RunManager"));
        let playground = GameGenre::Playground.scene_path();
        let (_, args, _) = UnityAction::SaveNamedScene.to_cli_args(
            "",
            &PathBuf::from(r"C:\Unity\P"),
            "Ground",
            GameGenre::Rpg.scene_path(),
        );
        let code = args.last().cloned().unwrap_or_default();
        assert!(code.contains(GameGenre::Rpg.scene_path()));
        let _ = playground;
    }

    #[test]
    fn enable_npc_ai_queues_install_reload_attach() {
        let mut state = UnityState::default();
        state.status = CliStatus::Missing;
        state.demo_mode = true;
        state.run_action(UnityAction::EnableNpcAi);
        assert_eq!(state.guide_kind, GuideKind::NpcAi);
        assert_eq!(state.guide_total, 3);
        assert_eq!(state.guide_queue[0], UnityAction::InstallNpcAi);
        assert_eq!(state.guide_queue[1], UnityAction::RequestScriptReload);
        assert_eq!(state.guide_queue[2], UnityAction::AttachNpcAi);
        assert_eq!(
            parse_unity_chat_command("给npc接入ai").unwrap().action,
            UnityAction::EnableNpcAi
        );
        let code = crate::npc_ai::eval_install_npc_ai_scripts();
        assert!(code.contains("BonyNpcBrain.cs"));
        assert!(code.contains("BonyNpcDialogue.cs"));
        assert!(crate::npc_ai::SCRIPT_NPC_BRAIN.contains("BonyNpcBrain"));
        assert!(crate::npc_ai::SCRIPT_NPC_DIALOGUE.contains("api.x.ai"));
        assert!(crate::npc_ai::EVAL_ATTACH_NPC_AI.contains("NPC_"));
        assert!(crate::npc_ai::EVAL_ATTACH_NPC_AI.contains("BonyNpcDialogue"));
    }

    #[test]
    fn routes_npc_and_marker_natural_language() {
        assert_eq!(
            parse_unity_chat_command("创建npc").unwrap().action,
            UnityAction::CreateNpc
        );
        assert_eq!(
            parse_unity_chat_command("创建商人").unwrap().action,
            UnityAction::CreateNpcVendor
        );
        assert_eq!(
            parse_unity_chat_command("创建任务npc").unwrap().action,
            UnityAction::CreateNpcQuest
        );
        assert_eq!(
            parse_unity_chat_command("创建出生点").unwrap().action,
            UnityAction::CreateSpawnPoint
        );
        assert_eq!(
            parse_unity_chat_command("创建传送门").unwrap().action,
            UnityAction::CreatePortalZone
        );
        assert_eq!(
            parse_unity_chat_command("创建敌人点").unwrap().action,
            UnityAction::CreateEnemySpawn
        );
        let (label, eval) = compile_unity_scene_command("创建3个npc").unwrap();
        assert_eq!(label, "创建 3 个 NPC");
        assert!(eval.contains("NPC_"));
        assert!(eval.contains("i < 3") || eval.contains("n < 3"));
        assert!(EVAL_CREATE_NPC.contains("NPC_"));
        assert!(EVAL_CREATE_NPC_VENDOR.contains("NPC_Vendor"));
        assert!(EVAL_CREATE_NPC_QUEST.contains("NPC_Quest"));
        assert!(EVAL_CREATE_SPAWN_POINT.contains("Spawn_"));
        assert!(EVAL_CREATE_PORTAL_ZONE.contains("Portal_Zone"));
        assert!(EVAL_CREATE_ENEMY_SPAWN.contains("Enemy_Spawn_"));
    }

    #[test]
    fn routes_scaffold_and_sky_natural_language() {
        let cmd = parse_unity_chat_command("帮我搭一个基础场景").unwrap();
        assert_eq!(cmd.action, UnityAction::ScaffoldMiniGame);
        assert_eq!(
            parse_unity_chat_command("做一个rpg").unwrap().action,
            UnityAction::ScaffoldRpg
        );
        assert_eq!(
            parse_unity_chat_command("搭mmo大厅").unwrap().action,
            UnityAction::ScaffoldMmo
        );
        assert_eq!(
            parse_unity_chat_command("创建肉鸽关卡").unwrap().action,
            UnityAction::ScaffoldRoguelike
        );
        assert_eq!(
            parse_unity_chat_command("roguelike雏形").unwrap().action,
            UnityAction::ScaffoldRoguelike
        );
        let (label, eval) = compile_unity_scene_command("换个蓝天").unwrap();
        assert!(label.contains("白天"));
        assert!(eval.contains("RenderSettings"));
        let (label, eval) = compile_unity_scene_command("来个晚霞天空").unwrap();
        assert!(label.contains("晚霞"));
        assert!(eval.contains("RenderSettings"));
        let (_, args, _) = UnityAction::SetupSkyDay.to_cli_args(
            "",
            &PathBuf::from(r"C:\Unity\P"),
            "Ground",
            GameGenre::Playground.scene_path(),
        );
        assert!(args.iter().any(|a| a == "eval"));
        assert!(args.iter().any(|a| a.contains("RenderSettings") || a == EVAL_SETUP_SKY_DAY));
    }

    #[test]
    fn resolve_root_from_assets_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("Assets").join("Skyboxes")).unwrap();
        std::fs::create_dir_all(root.join("ProjectSettings")).unwrap();
        let nested = root.join("Assets").join("Skyboxes");
        let resolved = resolve_unity_project_root(&nested).unwrap();
        assert_eq!(
            resolved.canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[test]
    fn unity_cli_path_drops_windows_extended_prefix() {
        let path = PathBuf::from(r"\\?\C:\Users\测试\UnityProject");
        #[cfg(windows)]
        assert_eq!(path_for_unity_cli(&path), r"C:\Users\测试\UnityProject");
        #[cfg(not(windows))]
        assert_eq!(path_for_unity_cli(&path), path.display().to_string());
    }

    #[test]
    fn pipeline_summary_distinguishes_package_from_live_server() {
        let raw = "Project Path PID Running Pipeline Version Update Available Server Port Server Reachable Safe Mode\n教程 C:\\Unity\\教程 123 true true 0.3.1-exp.1 false 0 false false";
        let summary = summarize_pipeline_list(raw);
        assert!(summary.contains("0.3.1-exp.1"));
        assert!(summary.contains("尚未启动"));
    }

    #[test]
    fn detects_manifest_only_pipeline_install() {
        let tmp = tempfile::tempdir().unwrap();
        let packages = tmp.path().join("Packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::write(
            packages.join("manifest.json"),
            r#"{"dependencies":{"com.unity.pipeline":"0.3.1-exp.1"}}"#,
        )
        .unwrap();
        assert!(pipeline_declared(&tmp.path().to_path_buf()));
        assert!(!pipeline_loaded_by_editor(&tmp.path().to_path_buf()));
    }

    #[test]
    fn detects_reachable_pipeline_with_wrapped_header_and_missing_pid() {
        let raw = "Project Path PID Running Pipeline Version Update Available Server Port Server\nReachable Safe Mode\nTutorial C:\\Unity\\Tutorial true true 0.3.1-exp.1 false 7800 true";
        assert_eq!(pipeline_server_reachable(raw), Some(true));
        let summary = summarize_pipeline_list(raw);
        assert!(summary.contains("0.3.1-exp.1"));
    }

    #[test]
    fn detects_unreachable_pipeline_using_port_anchor() {
        let raw = "Project Path PID Running Pipeline Version Update Available Server Port Server Reachable Safe Mode\nTutorial C:\\Unity\\Tutorial 123 true true 0.3.1-exp.1 false 7800 false false";
        assert_eq!(pipeline_server_reachable(raw), Some(false));
    }

    #[test]
    fn routes_scene_sphere_request_to_unity_eval() {
        let cmd = parse_unity_chat_command("帮我在场景画一个球体").unwrap();
        assert_eq!(cmd.action, UnityAction::Eval);
        assert!(cmd.eval.unwrap().contains("CreatePrimitive"));
        assert_eq!(cmd.slash, "/unity sphere");
    }

    #[test]
    fn compiles_parameterized_scene_creation() {
        let (label, eval) = compile_unity_scene_command("帮我创建3个球体并排放置").unwrap();
        assert_eq!(label, "创建 3 个球体");
        assert!(eval.contains("i < 3"));
        assert!(eval.contains("PrimitiveType.Sphere"));

        let (_, cube_eval) = compile_unity_scene_command("生成五个立方体").unwrap();
        assert!(cube_eval.contains("i < 5"));
        assert!(cube_eval.contains("PrimitiveType.Cube"));
    }

    #[test]
    fn compiles_follow_up_color_edit_for_selection() {
        let (label, eval) = compile_unity_scene_command("帮我补上绿色").unwrap();
        assert_eq!(label, "把选中对象设为绿色");
        assert!(eval.contains("UnityEditor.Selection.gameObjects"));
        assert!(eval.contains("Color.green"));
        assert!(eval.contains("GetComponentsInChildren<Renderer>"));
    }

    #[test]
    fn compiles_common_selection_edits() {
        assert!(
            compile_unity_scene_command("把它们向上移动")
                .unwrap()
                .1
                .contains("Vector3.up")
        );
        assert!(
            compile_unity_scene_command("把这些放大")
                .unwrap()
                .1
                .contains("2f")
        );
        assert!(
            compile_unity_scene_command("给它们添加刚体")
                .unwrap()
                .1
                .contains("Rigidbody")
        );
        assert!(
            compile_unity_scene_command("复制选中对象")
                .unwrap()
                .1
                .contains("Instantiate")
        );
        assert!(
            compile_unity_scene_command("删除选中对象")
                .unwrap()
                .1
                .contains("DestroyObjectImmediate")
        );
    }

    #[test]
    fn accepts_safe_generated_unity_plan() {
        let raw = r#"{"summary":"创建灯光","csharp":"var go = new GameObject(\"Light\"); UnityEditor.Undo.RegisterCreatedObjectUndo(go, \"Create Light\"); go.AddComponent<Light>(); return go.name;"}"#;
        let (summary, csharp) = parse_generated_unity_plan(raw).unwrap();
        assert_eq!(summary, "创建灯光");
        assert!(csharp.contains("AddComponent<Light>"));
    }

    #[test]
    fn rejects_generated_plan_with_external_io() {
        let raw = r#"{"summary":"write","csharp":"System.IO.File.WriteAllText(\"x\", \"y\"); return true;"}"#;
        assert!(parse_generated_unity_plan(raw).is_err());
    }

    #[test]
    fn eval_uses_named_code_argument_and_json_output() {
        let (_, args, _) =
            UnityAction::Eval.to_cli_args(
                "return 42;",
                &PathBuf::from(r"C:\Unity\Project"),
                "Ground",
                GameGenre::Playground.scene_path(),
            );
        assert_eq!(&args[0..2], &["--format", "json"]);
        assert!(args.windows(2).any(|pair| pair == ["--code", "return 42;"]));
        assert!(args.iter().any(|arg| arg == "--"));
    }

    #[test]
    fn eval_requires_structured_success() {
        assert!(eval_output_succeeded(
            r#"{"success":true,"data":{"success":true,"result":"Created 30 Cube"}}"#
        ));
        assert!(!eval_output_succeeded(
            r#"{"success":true,"data":{"success":false,"result":"compile error"}}"#
        ));
        assert!(!eval_output_succeeded("Command Success Result Parameters"));
    }
}
