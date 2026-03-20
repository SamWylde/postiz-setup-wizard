export function friendlyError(raw: string): string {
  const lower = raw.toLowerCase();

  if (lower.includes("error during connect") || lower.includes("is docker running")) {
    return "Docker Desktop doesn't seem to be running. Please start Docker Desktop and try again.";
  }
  if (lower.includes("network") && lower.includes("timeout")) {
    return "Network timeout. Please check your internet connection and try again.";
  }
  if (lower.includes("no space left") || lower.includes("disk full")) {
    return "Not enough disk space. Please free up some space and try again.";
  }
  if (lower.includes("permission denied") || lower.includes("access denied")) {
    return "Permission denied. Try running the app as administrator.";
  }
  if (lower.includes("pull") && lower.includes("fail")) {
    return "Failed to download Docker images. Make sure Docker Desktop is running and you're connected to the internet.";
  }
  if (lower.includes("cloudflared") && lower.includes("not found")) {
    return "cloudflared is not installed. Please install it first from the Prepare Computer step.";
  }
  if (lower.includes("tunnel") && (lower.includes("fail") || lower.includes("error"))) {
    return "Failed to create the web link. Make sure cloudflared is installed and you have internet access.";
  }
  if (lower.includes("compose") && lower.includes("not found")) {
    return "Docker Compose is not available. Make sure Docker Desktop is installed with Compose support.";
  }

  return raw;
}
