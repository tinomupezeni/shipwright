# 🚢 Shipwright

**Shipwright** is a server-centric deployment engine that transforms your VPS into a personal Mini-PaaS. It automates building, deploying, and monitoring your applications with zero manual configuration.

## ✨ Features

- **Mini-PaaS Architecture**: Build and deploy directly on your VPS—no more waiting for heavy Docker images to push/pull.
- **Auto-Provisioned CI/CD**: One command to link your GitHub repo and set up automatic deployments via webhooks.
- **Real-Time Observability**: Stream live build logs and system metrics from your VPS directly to your local terminal.
- **Zero-Downtime Deploys**: Smoothly swap containers upon successful builds.

## 🚀 Quick Start

### 1. Install Shipwright

Install the Shipwright CLI on your local machine (macOS/Linux):

```bash
curl -fsSL https://raw.githubusercontent.com/username/shipwright/main/scripts/install.sh | bash
```

### 2. Setup your VPS

Run this command once to prepare your VPS (installs Docker and the Shipwright Daemon):

```bash
shipwright setup
```

### 3. Initialize a Project

Go to your project directory and run:

```bash
shipwright init
```

### 4. Link to GitHub

Enable automatic deployments on every `git push`:

```bash
shipwright register
```

### 5. Watch the Magic

Push your code to GitHub and watch the build progress in real-time:

```bash
git push origin main
shipwright watch
```

## 🛠️ Architecture

Shipwright consists of two main components:
- **CLI**: The local orchestrator you use to manage projects and view logs.
- **Agent**: A global daemon running on your VPS that handles webhooks, clones code, and executes Docker builds.

## 📄 License

Apache 2.0 or MIT.
