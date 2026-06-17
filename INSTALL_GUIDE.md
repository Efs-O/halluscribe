# HalluScribe — Installation Guide

> HalluScribe requires a local AI backend to run Gemma 4 26B.
> Two options: **Ollama** (recommended, easiest) or **llama-server** (advanced).
> The Gemma model is large (~15GB). Ensure you have sufficient disk space and RAM/VRAM.

---

## Windows

### Option A — Ollama (Recommended)

1. Download and install Ollama from https://ollama.com
2. Open a terminal and run:
   ```
   ollama pull gemma4:26b
   ```
3. Ollama starts automatically as a background service on `localhost:11434`
4. Open HalluScribe → SETTINGS → set Backend to **Ollama**
5. Leave Host as `localhost`, Port as `11434`, Model tag as `gemma4:26b`
6. Done

**GPU:** Ollama detects your NVIDIA GPU automatically if CUDA drivers are installed.
CUDA drivers ship with the standard NVIDIA driver — no separate install needed in most cases.
If unsure, download the latest driver from https://www.nvidia.com/drivers

**CPU fallback:** Ollama runs on CPU if no GPU is detected. Inference will be slow
(minutes per response) but functional.

---

### Option B — llama-server (Advanced)

> Use this if you want full control over GPU layers, context size, and quantisation.

1. Download llama-server **build b8783 or newer** from the llama.cpp releases page.
   ⚠️ Earlier builds do not support Gemma 4 tool-calling — do not use them.
2. Download a Gemma 4 26B GGUF model file. Recommended quantisation: `Q3_K_M` or `Q4_K_M`.
   Available at: https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF
3. Open HalluScribe → SETTINGS → set Backend to **llama.cpp**
4. Set the llama-server binary path and model file path
5. Done — HalluScribe manages the server lifecycle automatically

---

## macOS (Apple Silicon — M1/M2/M3/M4)

### Option A — Ollama (Recommended)

1. Download and install Ollama from https://ollama.com (native Apple Silicon build available)
2. Open Terminal and run:
   ```
   ollama pull gemma4:26b
   ```
3. Open HalluScribe → SETTINGS → set Backend to **Ollama**
4. Leave Host as `localhost`, Port as `11434`, Model tag as `gemma4:26b`
5. Done

**GPU:** Ollama uses Apple Metal automatically on Apple Silicon — no drivers to install.
Unified memory means the model shares RAM between CPU and GPU seamlessly.

**Memory requirements:**
- Gemma 4 26B (default quantisation): ~14GB unified memory minimum
- M4 Pro 24GB: runs 26B at Q4 comfortably with 8K–16K context
- M4 base 16GB: runs 26B but system memory will be tight — expect slowdowns
- M1/M2 base 8GB: not enough — use `gemma4:8b` instead (~9.6GB at Q4_K_M)

### Option B — llama-server on Mac

> ✅ **Confirmed working** as of April 2026. llama.cpp added Gemma 4 support at
> model launch (April 2, 2026). Metal is enabled by default on macOS — no extra flags needed.

1. Download llama-server b8783+ (pre-built macOS binary or build from source)
2. Metal support is on by default — no `-DGGML_METAL` flag needed
3. Follow the same steps as Windows Option B for paths and SETTINGS
4. Recommended build flag if compiling from source: `-DGGML_CUDA=OFF` (Metal used instead)

---

## Linux

### Option A — Ollama (Recommended)

1. Install Ollama:
   ```
   curl -fsSL https://ollama.com/install.sh | sh
   ```
2. Pull the model:
   ```
   ollama pull gemma4:26b
   ```
3. Open HalluScribe → SETTINGS → set Backend to **Ollama**
4. Done

**NVIDIA GPU:** Ollama detects CUDA automatically if the NVIDIA driver is installed.
Install the driver via your distribution's package manager or from https://www.nvidia.com/drivers

**AMD GPU:** Ollama supports AMD GPUs via ROCm on Linux. Install ROCm first:
https://rocm.docs.amd.com — then Ollama will use it automatically.

**CPU fallback:** Works without a GPU, but inference will be slow.

### Option B — llama-server (Advanced)

Same as Windows Option B. Download b8783+ binary for Linux, or build from source.
CUDA and ROCm builds are both available in the llama.cpp releases.

---

## Quick Comparison

| | Ollama | llama-server |
|---|---|---|
| Setup difficulty | Easy | Advanced |
| Windows NVIDIA | ✅ Auto | ✅ Manual flags |
| Mac Apple Silicon | ✅ Metal auto (MLX since Mar 2026) | ✅ Metal default, confirmed Apr 2026 |
| Linux NVIDIA | ✅ Auto | ✅ Manual flags |
| Linux AMD | ✅ ROCm auto | ✅ ROCm manual |
| CPU fallback | ✅ Yes (slow) | ✅ Yes (slow) |
| Model management | `ollama pull` | Manual GGUF download |

---

## Minimum Hardware

| Component | Minimum | Recommended |
|---|---|---|
| RAM / Unified Memory | 16GB | 32GB+ |
| VRAM (dedicated GPU) | 12GB | 16GB+ |
| Disk space | 20GB free | 50GB free |
| CPU | Any modern x86-64 or Apple Silicon | — |

---

> **Note:** HalluScribe does not install Ollama, llama-server, CUDA, or Gemma models.
> These must be set up before launching HalluScribe.
> Once configured, HalluScribe manages the inference server lifecycle automatically
> — you do not need to start or stop it manually.
