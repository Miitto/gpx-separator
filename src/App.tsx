import { createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/tauri";
import { message, open } from "@tauri-apps/api/dialog";
import "./App.css";
import { UnlistenFn, listen } from "@tauri-apps/api/event";

function App() {
  const [listener, setListener] = createSignal<UnlistenFn | null>(null);
  const [processed, setProcessed] = createSignal<string[]>([]);

  async function pickFile() {
    let file = await open({
      directory: false,
      multiple: false,
      title: "Select GPX to Separate",
      filters: [{ name: "GPX Files", extensions: ["gpx"] }],
    }) as string | undefined;

    if (file) {
      console.log(file);
      try {
        await invoke("convert", { path: file });
      } catch (e) {
        console.error(e);
        message((e as {message: string}).message,
          {
          title: "Error",
          type: "error",
          }
        );
      }
    }
  }

  onMount(async () => {
    let unlistener = await listen('written', (event: {payload: {path: string}}) => {
      event.payload.path && setProcessed((prev) => [...prev, event.payload.path]);
    });
    setListener(() => unlistener);
  });

  onCleanup(() => {
    listener() && listener()!();
  });

  async function openFile(file: string) {
    try {
      await invoke("open_file", { path: file + ".gpx" });
    }catch (e) {
      console.error(e);
        message((e as {message: string}).message,
          {
          title: "Error",
          type: "error",
          }
        );
  }
}

  return (
    <div class="container">
      <button onClick={pickFile}>Pick</button>
      <h1>Processed files</h1>
      <ul>
        {processed().map((file) => (
          <li>{file}<button onClick={() => openFile(file)}>View</button></li>
        ))}
      </ul>
    </div>
  );
}

export default App;
