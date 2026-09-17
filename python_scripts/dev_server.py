"""Loopback-only development server for the ReMinori Web program files.

Serves the browser shell from `crates/web/static` under `/static/*` and the
wasm-bindgen output from `target/wasm-web/pkg` under `/pkg/*`, so the fixed
URLs baked into `bootstrap.js` (`/pkg/eden_web.js`, ...) resolve. The shell
only accepts the fixed origin `http://127.0.0.1:4173`.
"""

from __future__ import annotations

import argparse
import socket
import threading
import webbrowser
from pathlib import Path

import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse, JSONResponse


HOST = "127.0.0.1"
PORT = 4173
SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE_ROOT = SCRIPT_DIR.parents[0]
STATIC_DIR = (WORKSPACE_ROOT / "crates" / "web" / "static").resolve()
PKG_DIR = (WORKSPACE_ROOT / "target" / "wasm-web" / "pkg").resolve()


def _assert_workspace_child(path: Path) -> None:
    try:
        path.relative_to(WORKSPACE_ROOT)
    except ValueError as error:
        raise RuntimeError(f"unsafe development path: {path}") from error


_assert_workspace_child(STATIC_DIR)
_assert_workspace_child(PKG_DIR)
if not STATIC_DIR.is_dir():
    raise RuntimeError(f"Web static directory is missing: {STATIC_DIR}")

app = FastAPI(
    title="ReMinori Web development server",
    docs_url=None,
    redoc_url=None,
    openapi_url=None,
)


@app.middleware("http")
async def disable_development_cache(request, call_next):
    response = await call_next(request)
    response.headers["Cache-Control"] = "no-store"
    return response


@app.get("/__dev/health")
def health() -> JSONResponse:
    files = {
        "eden_web.js": (PKG_DIR / "eden_web.js").is_file(),
        "eden_web_bg.wasm": (PKG_DIR / "eden_web_bg.wasm").is_file(),
    }
    return JSONResponse(
        {
            "status": "ready" if all(files.values()) else "build-required",
            "build_dir": "target/wasm-web/pkg",
            "files": files,
            "version": 1,
        }
    )


@app.get("/")
def index() -> FileResponse:
    return FileResponse(STATIC_DIR / "index.html", media_type="text/html")


@app.get("/static/{filename}")
def static_file(filename: str) -> FileResponse:
    allowed = {
        "app.css": "text/css",
        "bootstrap.js": "text/javascript",
    }
    media_type = allowed.get(filename)
    if media_type is None:
        raise HTTPException(status_code=404)
    return FileResponse(STATIC_DIR / filename, media_type=media_type)


@app.get("/pkg/{filename}")
def package_file(filename: str) -> FileResponse:
    allowed = {
        "eden_web.js": "text/javascript",
        "eden_web_bg.wasm": "application/wasm",
    }
    media_type = allowed.get(filename)
    path = PKG_DIR / filename
    if media_type is None or not path.is_file():
        raise HTTPException(status_code=404)
    return FileResponse(path, media_type=media_type)


def _ensure_port_available() -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        try:
            probe.bind((HOST, PORT))
        except OSError as error:
            raise SystemExit(
                f"Cannot start ReMinori Web server: {HOST}:{PORT} is already in use. "
                "Stop the process using the fixed development port and retry."
            ) from error


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--open", action="store_true", help="open the default browser")
    args = parser.parse_args()
    _ensure_port_available()
    if args.open:
        threading.Timer(0.5, webbrowser.open, args=(f"http://{HOST}:{PORT}",)).start()
    uvicorn.run(app, host=HOST, port=PORT, reload=False, access_log=True)


if __name__ == "__main__":
    main()
