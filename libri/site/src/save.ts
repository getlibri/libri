// Saving a file the page made (an export, a picture) to the device.
//
// A browser takes a link with `download`. The Android app's web view can't:
// its download handler sees only addresses, and a `blob:` one is this
// page's alone. There the page hands the file to the app (`LibriApp`, the
// app's message channel to the host's own page), which saves it to
// Downloads.

type App = { postMessage: (m: string) => void };

/** The app's channel, when this page runs inside the Android app. */
const app = () => (window as unknown as { LibriApp?: App }).LibriApp;

/** Saves `blob` as `name`, in the browser or through the app. */
export async function saveFile(blob: Blob, name: string): Promise<void> {
  const a = app();
  if (a) {
    // As base64 by the browser's own reader: building it from the bytes in
    // script spreads thousands of arguments, past what a phone's web view
    // allows on its stack.
    const url = await new Promise<string>((done, fail) => {
      const r = new FileReader();
      r.onload = () => done(String(r.result));
      r.onerror = () => fail(r.error);
      r.readAsDataURL(blob);
    });
    a.postMessage(JSON.stringify({ save: { name, type: blob.type || "application/octet-stream", data: url.slice(url.indexOf(",") + 1) } }));
    return;
  }
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 1000);
}
