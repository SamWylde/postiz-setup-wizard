export interface InstructionStep {
  text: string;
  copyValue?: string;
  copyLabel?: string;
}

export interface ProviderDefinition {
  id: string;
  name: string;
  icon: string;
  docsUrl: string;
  portalUrl: string;
  envKeys: { key: string; label: string; secret: boolean }[];
  callbackUrlTemplate: string;
  homepageUrlTemplate?: string;
  instructions: InstructionStep[];
  requiresHttps: boolean;
  sharedWith?: string;
  noEnvNeeded?: boolean;
  gated?: string;
  popular?: boolean;
  requiresPermanentDomain?: boolean;
  temporaryLinkNote?: string;
  supportsLocalCallback?: boolean;
}

export const providers: ProviderDefinition[] = [
  {
    id: "facebook",
    name: "Facebook",
    icon: "Facebook",
    popular: true,
    docsUrl: "https://docs.postiz.com/providers/facebook",
    portalUrl: "https://developers.facebook.com/apps",
    envKeys: [
      { key: "FACEBOOK_APP_ID", label: "App ID", secret: false },
      { key: "FACEBOOK_APP_SECRET", label: "App Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/facebook",
    homepageUrlTemplate: "{baseUrl}/",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      { text: "Go to Meta for Developers and click 'Create App'" },
      { text: "Select 'Other' as the use case, then choose 'Business'" },
      { text: "Enter an app name (e.g. 'Postiz Social') and create the app" },
      { text: "Add 'Login for Business' product to your app" },
      {
        text: "In Login settings, add this redirect URI:",
        copyLabel: "Redirect URI",
      },
      {
        text: "Go to App Settings > Basic, add your website URL:",
        copyLabel: "Website URL",
      },
      {
        text: "Go to App Review > Permissions and Features, then request: pages_show_list, business_management, pages_manage_posts, pages_manage_engagement, pages_read_engagement, read_insights",
      },
      { text: "Switch the app to Live mode" },
      { text: "Copy your App ID and App Secret from App Settings > Basic" },
    ],
  },
  {
    id: "instagram",
    name: "Instagram",
    icon: "Instagram",
    docsUrl: "https://docs.postiz.com/providers/instagram",
    portalUrl: "https://developers.facebook.com/apps",
    envKeys: [
      {
        key: "FACEBOOK_APP_ID",
        label: "Facebook App ID (same as Facebook)",
        secret: false,
      },
      {
        key: "FACEBOOK_APP_SECRET",
        label: "Facebook App Secret (same as Facebook)",
        secret: true,
      },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/instagram",
    requiresHttps: false,
    sharedWith: "facebook",
    supportsLocalCallback: true,
    instructions: [
      {
        text: "Use the same Meta app as Facebook (or create a new one)",
      },
      {
        text: "Add 'Instagram Business Login' product to your app",
      },
      {
        text: "Add this redirect URI in Instagram Login settings:",
        copyLabel: "Redirect URI",
      },
      {
        text: "Go to App Review > Permissions and Features, then request: instagram_basic, instagram_content_publish, instagram_manage_comments, instagram_manage_insights, pages_show_list, pages_read_engagement, business_management",
      },
      {
        text: "Add your Instagram account as an app tester and accept the invitation",
      },
    ],
  },
  {
    id: "linkedin",
    name: "LinkedIn",
    icon: "Linkedin",
    popular: true,
    docsUrl: "https://docs.postiz.com/providers/linkedin",
    portalUrl: "https://www.linkedin.com/developers/apps",
    envKeys: [
      { key: "LINKEDIN_CLIENT_ID", label: "Client ID", secret: false },
      { key: "LINKEDIN_CLIENT_SECRET", label: "Client Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/linkedin",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      { text: "Go to LinkedIn Developers and click 'Create App'" },
      {
        text: "Fill in the app name, associate with your company page, and upload a logo",
      },
      { text: "Go to the Products tab and request 'Share on LinkedIn' and 'Sign In with LinkedIn using OpenID Connect'" },
      {
        text: "Also request 'Advertising API' in the Products tab (needed for token refreshing)",
      },
      {
        text: "Go to Auth tab and add this redirect URL:",
        copyLabel: "Redirect URL",
      },
      {
        text: "Copy your Client ID and Client Secret from the Auth tab",
      },
    ],
  },
  {
    id: "x",
    name: "X (Twitter)",
    icon: "Twitter",
    popular: true,
    docsUrl: "https://docs.postiz.com/providers/x-twitter",
    portalUrl: "https://developer.x.com",
    envKeys: [
      { key: "X_API_KEY", label: "API Key", secret: false },
      { key: "X_API_SECRET", label: "API Key Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/x",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      { text: "Go to X Developer Portal and sign up for the Free tier" },
      { text: "Create a Project, then create an App inside it" },
      {
        text: "Go to User Authentication Settings > Set up",
      },
      {
        text: "Set App Permissions to 'Read and Write', App type to 'Native App'",
      },
      {
        text: "Add this Callback URL:",
        copyLabel: "Callback URL",
      },
      {
        text: "Go to Keys and Tokens > Regenerate Consumer Keys",
      },
      { text: "Copy the API Key and API Key Secret" },
    ],
  },
  {
    id: "reddit",
    name: "Reddit",
    icon: "MessageCircle",
    docsUrl: "https://docs.postiz.com/providers/reddit",
    portalUrl: "https://www.reddit.com/prefs/apps",
    envKeys: [
      { key: "REDDIT_CLIENT_ID", label: "Client ID", secret: false },
      { key: "REDDIT_CLIENT_SECRET", label: "Client Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/reddit",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      {
        text: "Go to Reddit App Preferences and click 'create another app'",
      },
      { text: "Enter a name (e.g. 'Postiz'), select 'web app' type" },
      {
        text: "Set this redirect URI:",
        copyLabel: "Redirect URI",
      },
      {
        text: "Copy the client ID (shown under the app name) and the secret",
      },
    ],
  },
  {
    id: "threads",
    name: "Threads",
    icon: "AtSign",
    docsUrl: "https://docs.postiz.com/providers/threads",
    portalUrl: "https://developers.facebook.com/apps",
    envKeys: [
      { key: "THREADS_APP_ID", label: "Threads App ID", secret: false },
      { key: "THREADS_APP_SECRET", label: "Threads App Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/threads",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      { text: "Go to Meta for Developers (use same or new app)" },
      { text: "Add 'Access the Threads API' product" },
      {
        text: "Go to Use Cases > Authenticate and request data from users, then add: threads_content_publish, threads_basic",
      },
      {
        text: "Set this redirect URI (click the URL to make it active):",
        copyLabel: "Redirect URI",
      },
      {
        text: "Add your Threads account as a tester in App Roles and accept the invite via Threads settings",
      },
      { text: "Copy the Threads App ID and App Secret" },
    ],
  },
  {
    id: "youtube",
    name: "YouTube",
    icon: "Youtube",
    popular: true,
    docsUrl: "https://docs.postiz.com/providers/youtube",
    portalUrl: "https://console.cloud.google.com/apis/credentials",
    envKeys: [
      { key: "YOUTUBE_CLIENT_ID", label: "Client ID", secret: false },
      { key: "YOUTUBE_CLIENT_SECRET", label: "Client Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/youtube",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      { text: "Go to Google Cloud Console > APIs & Services > Credentials" },
      { text: "Create a new project if needed" },
      { text: "Set up the OAuth consent screen" },
      { text: "Create an OAuth 2.0 Client ID (Web application type)" },
      {
        text: "Add this redirect URI:",
        copyLabel: "Redirect URI",
      },
      {
        text: "Go to APIs & Services > Library, search for and enable: YouTube Data API v3, YouTube Analytics API, YouTube Reporting API",
      },
      { text: "Add yourself as a test user" },
      { text: "Copy the Client ID and Client Secret" },
    ],
  },
  {
    id: "tiktok",
    name: "TikTok",
    icon: "Music",
    docsUrl: "https://docs.postiz.com/providers/tiktok",
    portalUrl: "https://developers.tiktok.com/apps",
    envKeys: [
      { key: "TIKTOK_CLIENT_ID", label: "Client ID", secret: false },
      { key: "TIKTOK_CLIENT_SECRET", label: "Client Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/tiktok",
    requiresHttps: true,
    requiresPermanentDomain: true,
    supportsLocalCallback: false,
    instructions: [
      { text: "Go to TikTok for Developers and create an app" },
      { text: "Add Login Kit and Content Posting API products" },
      {
        text: "In your app's Manage > Scopes section, add: user.info.basic, video.create, video.publish, video.upload, user.info.profile",
      },
      {
        text: "Set this callback URL:",
        copyLabel: "Callback URL",
      },
      { text: "Copy the Client Key (16 chars) and Client Secret (32 chars)" },
    ],
  },
  {
    id: "pinterest",
    name: "Pinterest",
    icon: "Pin",
    docsUrl: "https://docs.postiz.com/providers/pinterest",
    portalUrl: "https://developers.pinterest.com/apps/",
    envKeys: [
      { key: "PINTEREST_CLIENT_ID", label: "App ID", secret: false },
      { key: "PINTEREST_CLIENT_SECRET", label: "App Secret", secret: true },
    ],
    callbackUrlTemplate: "{baseUrl}/integrations/social/pinterest",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      {
        text: "Go to Pinterest Developer Dashboard and create an app (requires a Pinterest Business account)",
      },
      { text: "Fill out all required information and wait for approval" },
      {
        text: "Add this redirect URI:",
        copyLabel: "Redirect URI",
      },
      { text: "Copy your App ID and App Secret" },
    ],
  },
  {
    id: "bluesky",
    name: "Bluesky",
    icon: "Cloud",
    docsUrl: "https://docs.postiz.com/providers/bluesky",
    portalUrl: "",
    envKeys: [],
    callbackUrlTemplate: "",
    requiresHttps: false,
    noEnvNeeded: true,
    supportsLocalCallback: true,
    instructions: [
      {
        text: "No developer setup needed! Connect Bluesky directly in the Postiz web interface using your app password.",
      },
    ],
  },
  {
    id: "discord",
    name: "Discord",
    icon: "MessageSquare",
    docsUrl: "https://docs.postiz.com/providers/discord",
    portalUrl: "https://discord.com/developers/applications",
    envKeys: [
      { key: "DISCORD_CLIENT_ID", label: "Client ID", secret: false },
      { key: "DISCORD_CLIENT_SECRET", label: "Client Secret", secret: true },
      { key: "DISCORD_BOT_TOKEN_ID", label: "Bot Token", secret: true },
    ],
    callbackUrlTemplate: "",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      { text: "Go to Discord Developer Portal and create a new application" },
      { text: "Go to OAuth2 tab and copy Client ID and Client Secret" },
      { text: "Go to Bot tab, create a bot, and copy the token" },
    ],
  },
  {
    id: "mastodon",
    name: "Mastodon",
    icon: "Globe",
    docsUrl: "https://docs.postiz.com/providers/mastodon",
    portalUrl: "",
    envKeys: [
      {
        key: "MASTODON_URL",
        label: "Mastodon Instance URL",
        secret: false,
      },
      { key: "MASTODON_CLIENT_ID", label: "Client ID", secret: false },
      {
        key: "MASTODON_CLIENT_SECRET",
        label: "Client Secret",
        secret: true,
      },
    ],
    callbackUrlTemplate: "",
    requiresHttps: false,
    supportsLocalCallback: true,
    instructions: [
      {
        text: "On your Mastodon instance, go to Settings > Development > New Application",
      },
      { text: "Copy your Client ID and Client Secret" },
      {
        text: "Set the Mastodon instance URL (default: https://mastodon.social)",
      },
    ],
  },
];

export function getProvider(id: string): ProviderDefinition | undefined {
  return providers.find((p) => p.id === id);
}

export function isEphemeralTunnelUrl(url: string): boolean {
  return (
    url.includes("trycloudflare.com") ||
    url.includes("ngrok-free.app") ||
    url.includes("ngrok.io") ||
    url.includes(".zrok.io") ||
    url.includes(".pinggy.link") ||
    url.includes(".pinggy.io")
  );
}

export function getCallbackUrl(
  provider: ProviderDefinition,
  baseUrl: string,
): string {
  if (provider.requiresPermanentDomain && isEphemeralTunnelUrl(baseUrl)) {
    return "(requires permanent domain)";
  }
  return provider.callbackUrlTemplate.replace("{baseUrl}", baseUrl);
}

export function getHomepageUrl(
  provider: ProviderDefinition,
  baseUrl: string,
): string {
  return (provider.homepageUrlTemplate ?? "").replace("{baseUrl}", baseUrl);
}
