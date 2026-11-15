// A whiteboard alone, on a page of its own (board.html), so it loads the
// board and Excalidraw and never the whole app:
// - `#board-tile=<id>`: the Whiteboard tile, for the Android app's place of
//   it. The board picked is told to the app (`LibriApp`, as a
//   "board:<id>" message), which keeps it as that place's own.
// - `#board=<token>`: the board a link opens, for whoever holds it (served
//   by src/share.rs, which reads the token from a header, never the address
//   the browser sends).
// The app names its theme (`&theme=dark`) and paper (`&paper=E6D8C1`), as
// it does for a Library page alone.

import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter";
import "./index.css";
import { BoardTile, SharedBoard, boardMark } from "./board";
import { useTheme } from "./theme";

const asked = new URLSearchParams(location.hash.slice(1));
const tile = asked.get("board-tile");
const token = asked.get("board");
if (tile !== null) {
  document.documentElement.dataset.theme = asked.get("theme") === "dark" ? "dark" : "light";
  const paper = asked.get("paper");
  if (paper && /^[0-9a-f]{6}$/i.test(paper)) document.documentElement.style.setProperty("--paper", `#${paper}`);
}

function TileAlone({ first }: { first: string }) {
  const [id, setId] = useState(first || undefined);
  const pick = (b: string) => {
    setId(b);
    (window as unknown as { LibriApp?: { postMessage: (m: string) => void } }).LibriApp?.postMessage(`board:${b}`);
  };
  return (
    <div className="fixed inset-0 bg-paper">
      <BoardTile id={id} pick={pick} phone />
    </div>
  );
}

function Themed({ children }: { children: React.ReactNode }) {
  useTheme();
  return children;
}

boardMark("page");
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {token ? (
      <Themed>
        <SharedBoard token={token} />
      </Themed>
    ) : (
      <TileAlone first={tile ?? ""} />
    )}
  </StrictMode>,
);
