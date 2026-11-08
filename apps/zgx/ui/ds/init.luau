-- Zeron's design system, on zgx's primitives. Three layers, as zgx's
-- design guide has them: the theme (tokens.luau, themes/*.json) holds every
-- value; the components here turn tokens into styles with `ui.variants`;
-- the design lint (`ui.theme.strict`, on in every entry but the intro)
-- keeps screens from going around them.
--
-- Screens take what they need from here:
--   local ds = require("@ds")
--   ds.Text { "Welcome", style = "display" }

-- (First: the words' catalog, before any module makes a message.)
local locale = require("@self/locale")
local tokens = require("@self/tokens")
local mark = require("@self/mark")
local surface = require("@self/surface")

export type MarkName = mark.Name
export type SurfaceKind = surface.Kind
local member_card = require("@self/member_card")
local composer = require("@self/composer")
local model_picker = require("@self/model_picker")
local status = require("@self/status")
local sidebar = require("@self/sidebar")
export type SidebarSession = sidebar.Session
export type SidebarProject = sidebar.Project
export type StatusKind = status.Kind
export type Harness = model_picker.Harness
export type Model = model_picker.Model
export type ComposerDevice = composer.Device
export type ComposerUsage = composer.Usage
export type ComposerFixed = composer.Fixed
export type CardLayer = member_card.Layer
export type CardBadge = member_card.Badge
local card_wall = require("@self/card_wall")
local onboarding_frame = require("@self/onboarding_frame")
export type OnboardingStep = onboarding_frame.StepProps
export type OnboardingForward = onboarding_frame.Forward
export type OnboardingQuiet = onboarding_frame.Quiet
export type OnboardingForm = onboarding_frame.FormProps
export type CardMember = card_wall.Member
export type CardColumn = card_wall.Column

-- What it holds, for its types: the index as written (never called).
local function all()
	return {
		-- Loads the themes and declares the app's tokens: an entry calls it
		-- first, before it builds anything.
		setup = tokens.setup,
		-- Dates and times in the locale's words (ds/locale).
		Locale = locale,
		Text = require("@self/text"),
		Button = require("@self/button"),
		Surface = surface,
		Mark = mark,
		Icon = require("@self/icon"),
		MemberCard = member_card,
		-- The ways to share a card, as one control (under a card, or beside one).
		CardActions = require("@self/card_actions"),
		-- A card from a seed (a user id): its scene, foil and sky, as the
		-- backend and the web compute them; and the scenes a seed picks among.
		CardSeed = require("@self/card_seed"),
		-- Columns of member cards drifting beside a screen's words.
		CardWall = card_wall,
		CardScenes = require("@self/card_scenes"),
		Steps = require("@self/steps"),
		Switch = require("@self/switch"),
		Segmented = require("@self/segmented"),
		Dropdown = require("@self/dropdown"),
		Composer = composer,
		-- What shows as the pointer rests on something: every node tooltip's
		-- frame (`HoverCard.Card`, `HoverCard.Tip`).
		HoverCard = require("@self/hover_card"),
		-- How full a model's context is: the composer ring's card and its words.
		context_usage = require("@self/context_usage"),
		ModelPicker = model_picker,
		EffortSlider = require("@self/effort_slider"),
		RollText = require("@self/roll_text"),
		SidebarIcon = require("@self/sidebar_icon"),
		Sidebar = sidebar,
		-- A session's hover card (the sidebar's rest on a row; the shell places it).
		SessionCard = require("@self/session_card"),
		FadeText = require("@self/fade_text"),
		-- An email address that can be hidden, blurred (Settings → General).
		PrivateText = require("@self/private_text"),
		Tooltip = require("@self/tooltip"),
		AccountRow = require("@self/account_row"),
		Avatar = require("@self/avatar"),
		Tile = require("@self/tile"),
		TitleBar = require("@self/title_bar"),
		-- Controls and pieces every surface shares.
		IconButton = require("@self/icon_button").IconButton,
		ToggleButton = require("@self/toggle_button"),
		Menu = require("@self/menu"),
		Glass = require("@self/glass").Panel,
		glass = require("@self/glass").style,
		SectionTitle = require("@self/section_title"),
		Hint = require("@self/hint"),
		-- Primitives: fields, choices, notices, overlays, and the pieces
		-- settings, onboarding and the git pages are built from.
		Kbd = require("@self/kbd"),
		TextField = require("@self/text_field"),
		TextArea = require("@self/text_area"),
		SearchField = require("@self/search_field"),
		TokenField = require("@self/token_field"),
		Checkbox = require("@self/checkbox"),
		RadioGroup = require("@self/radio"),
		Select = require("@self/select"),
		Slider = require("@self/slider"),
		NumberStepper = require("@self/stepper"),
		DatePicker = require("@self/date_picker"),
		Tabs = require("@self/tabs"),
		Badge = require("@self/badge").Badge,
		Tag = require("@self/badge").Tag,
		Count = require("@self/badge").Count,
		Dot = require("@self/badge").Dot,
		PullRequestBadge = require("@self/badge").PullRequest,
		ReviewBadge = require("@self/badge").Review,
		ChecksBadge = require("@self/badge").Checks,
		FileStatus = require("@self/badge").FileStatus,
		DiffStat = require("@self/badge").DiffStat,
		AvatarStack = require("@self/people").AvatarStack,
		Person = require("@self/people").Person,
		Timestamp = require("@self/people").Timestamp,
		Callout = require("@self/banner").Callout,
		Banner = require("@self/banner").Banner,
		toast = require("@self/toast"),
		Dialog = require("@self/dialog").Dialog,
		Sheet = require("@self/dialog").Sheet,
		EmptyState = require("@self/empty_state"),
		Skeleton = require("@self/skeleton").Bar,
		SkeletonRow = require("@self/skeleton").Row,
		ProgressBar = require("@self/progress").ProgressBar,
		ProgressRing = require("@self/progress").ProgressRing,
		Divider = require("@self/layout").Divider,
		Link = require("@self/layout").Link,
		Breadcrumbs = require("@self/layout").Breadcrumbs,
		KeyValue = require("@self/layout").KeyValue,
		SplitButton = require("@self/split_button"),
		Toolbar = require("@self/toolbar").Toolbar,
		ToolbarGroup = require("@self/toolbar").Group,
		ToolbarSpacer = require("@self/toolbar").Spacer,
		CommentCard = require("@self/comment").CommentCard,
		CommentThread = require("@self/comment").Thread,
		-- What stands between two lines of code.
		InlineBlock = require("@self/inline_block").InlineBlock,
		InlineComposer = require("@self/inline_block").Composer,
		InlineThread = require("@self/inline_block").Thread,
		InlineNote = require("@self/inline_block").Note,
		InlineAdd = require("@self/inline_block").Add,
		ReplyComposer = require("@self/comment").ReplyComposer,
		Settings = require("@self/settings"),
		-- A new chat's wallpaper: a picture, or a plugin's, under an effect.
		Wallpaper = require("@self/wallpaper").Wallpaper,
		wallpaper = require("@self/wallpaper"),
		-- A project's files beside the one they show, the tree hideable.
		FilesPane = require("@self/files_pane"),
		-- The app's scrollbar, floating over a scroll view's trailing edge.
		Rail = require("@self/rail"),
		Hero = require("@self/onboarding").Hero,
		StepHeader = require("@self/onboarding").StepHeader,
		OptionCards = require("@self/onboarding").OptionCards,
		-- Setup's window and its steps: the frame, a step with its footer, the
		-- footer's pills, a step's form.
		Onboarding = onboarding_frame,
		-- Make it yours: the scheme, the window's material and the accent.
		AppearancePicker = require("@self/appearance_picker"),
		-- The organizations someone is in, and invitations, to choose one.
		OrgList = require("@self/org_list"),
		-- The address a name makes, and whether it's free.
		AddressLine = require("@self/address_line"),
		-- GitHub for Cloud agents: Settings → Cloud's block, in setup too.
		GitHubBlock = require("@self/github_block"),
		-- An agent on a device and its accounts (Settings → Providers' cards).
		ProviderCard = require("@self/provider_card"),
		-- An agent's mark: Zeron's own, a registry agent's icon, or its initial.
		AgentMark = require("@self/agent_mark"),
		-- The accent the app is drawn in, as the material is (ds.material).
		look = require("@self/look"),
		Status = status.Status,
		StatusDot = status.StatusDot,
		StatusIcon = status.StatusIcon,
		StatusTag = status.StatusTag,
		-- Effects.
		decode = require("@self/decode"),
		effects = require("@self/effects"),
		island = require("@self/island"),
		-- Frosted or opaque, for the whole app.
		material = require("@self/material"),
	}
end

-- (Each loads as a screen first takes it, not with the index: a launch builds few of them.)
local lazy: { [string]: () -> any } = {
	Text = function()
		return require("@self/text")
	end,
	Button = function()
		return require("@self/button")
	end,
	Icon = function()
		return require("@self/icon")
	end,
	CardActions = function()
		return require("@self/card_actions")
	end,
	CardSeed = function()
		return require("@self/card_seed")
	end,
	CardScenes = function()
		return require("@self/card_scenes")
	end,
	Steps = function()
		return require("@self/steps")
	end,
	Switch = function()
		return require("@self/switch")
	end,
	Segmented = function()
		return require("@self/segmented")
	end,
	Dropdown = function()
		return require("@self/dropdown")
	end,
	HoverCard = function()
		return require("@self/hover_card")
	end,
	context_usage = function()
		return require("@self/context_usage")
	end,
	EffortSlider = function()
		return require("@self/effort_slider")
	end,
	RollText = function()
		return require("@self/roll_text")
	end,
	SidebarIcon = function()
		return require("@self/sidebar_icon")
	end,
	SessionCard = function()
		return require("@self/session_card")
	end,
	FadeText = function()
		return require("@self/fade_text")
	end,
	PrivateText = function()
		return require("@self/private_text")
	end,
	Tooltip = function()
		return require("@self/tooltip")
	end,
	AccountRow = function()
		return require("@self/account_row")
	end,
	Avatar = function()
		return require("@self/avatar")
	end,
	Tile = function()
		return require("@self/tile")
	end,
	TitleBar = function()
		return require("@self/title_bar")
	end,
	IconButton = function()
		return require("@self/icon_button").IconButton
	end,
	ToggleButton = function()
		return require("@self/toggle_button")
	end,
	Menu = function()
		return require("@self/menu")
	end,
	Glass = function()
		return require("@self/glass").Panel
	end,
	glass = function()
		return require("@self/glass").style
	end,
	SectionTitle = function()
		return require("@self/section_title")
	end,
	Hint = function()
		return require("@self/hint")
	end,
	Kbd = function()
		return require("@self/kbd")
	end,
	TextField = function()
		return require("@self/text_field")
	end,
	TextArea = function()
		return require("@self/text_area")
	end,
	SearchField = function()
		return require("@self/search_field")
	end,
	TokenField = function()
		return require("@self/token_field")
	end,
	Checkbox = function()
		return require("@self/checkbox")
	end,
	RadioGroup = function()
		return require("@self/radio")
	end,
	Select = function()
		return require("@self/select")
	end,
	Slider = function()
		return require("@self/slider")
	end,
	NumberStepper = function()
		return require("@self/stepper")
	end,
	DatePicker = function()
		return require("@self/date_picker")
	end,
	Tabs = function()
		return require("@self/tabs")
	end,
	Badge = function()
		return require("@self/badge").Badge
	end,
	Tag = function()
		return require("@self/badge").Tag
	end,
	Count = function()
		return require("@self/badge").Count
	end,
	Dot = function()
		return require("@self/badge").Dot
	end,
	PullRequestBadge = function()
		return require("@self/badge").PullRequest
	end,
	ReviewBadge = function()
		return require("@self/badge").Review
	end,
	ChecksBadge = function()
		return require("@self/badge").Checks
	end,
	FileStatus = function()
		return require("@self/badge").FileStatus
	end,
	DiffStat = function()
		return require("@self/badge").DiffStat
	end,
	AvatarStack = function()
		return require("@self/people").AvatarStack
	end,
	Person = function()
		return require("@self/people").Person
	end,
	Timestamp = function()
		return require("@self/people").Timestamp
	end,
	Callout = function()
		return require("@self/banner").Callout
	end,
	Banner = function()
		return require("@self/banner").Banner
	end,
	toast = function()
		return require("@self/toast")
	end,
	Dialog = function()
		return require("@self/dialog").Dialog
	end,
	Sheet = function()
		return require("@self/dialog").Sheet
	end,
	EmptyState = function()
		return require("@self/empty_state")
	end,
	Skeleton = function()
		return require("@self/skeleton").Bar
	end,
	SkeletonRow = function()
		return require("@self/skeleton").Row
	end,
	ProgressBar = function()
		return require("@self/progress").ProgressBar
	end,
	ProgressRing = function()
		return require("@self/progress").ProgressRing
	end,
	Divider = function()
		return require("@self/layout").Divider
	end,
	Link = function()
		return require("@self/layout").Link
	end,
	Breadcrumbs = function()
		return require("@self/layout").Breadcrumbs
	end,
	KeyValue = function()
		return require("@self/layout").KeyValue
	end,
	SplitButton = function()
		return require("@self/split_button")
	end,
	Toolbar = function()
		return require("@self/toolbar").Toolbar
	end,
	ToolbarGroup = function()
		return require("@self/toolbar").Group
	end,
	ToolbarSpacer = function()
		return require("@self/toolbar").Spacer
	end,
	CommentCard = function()
		return require("@self/comment").CommentCard
	end,
	CommentThread = function()
		return require("@self/comment").Thread
	end,
	ReplyComposer = function()
		return require("@self/comment").ReplyComposer
	end,
	Settings = function()
		return require("@self/settings")
	end,
	Rail = function()
		return require("@self/rail")
	end,
	Hero = function()
		return require("@self/onboarding").Hero
	end,
	StepHeader = function()
		return require("@self/onboarding").StepHeader
	end,
	OptionCards = function()
		return require("@self/onboarding").OptionCards
	end,
	AppearancePicker = function()
		return require("@self/appearance_picker")
	end,
	OrgList = function()
		return require("@self/org_list")
	end,
	AddressLine = function()
		return require("@self/address_line")
	end,
	GitHubBlock = function()
		return require("@self/github_block")
	end,
	ProviderCard = function()
		return require("@self/provider_card")
	end,
	AgentMark = function()
		return require("@self/agent_mark")
	end,
	look = function()
		return require("@self/look")
	end,
	decode = function()
		return require("@self/decode")
	end,
	effects = function()
		return require("@self/effects")
	end,
	island = function()
		return require("@self/island")
	end,
}

-- (Their types are exported above, or a module too big to compile inside a handler's 50 ms, or `material`, whose effects must be the script's.)
local ds: { [string]: any } = {
	setup = tokens.setup,
	Locale = locale,
	Surface = surface,
	Mark = mark,
	MemberCard = member_card,
	CardWall = card_wall,
	Composer = composer,
	ModelPicker = model_picker,
	Sidebar = sidebar,
	InlineBlock = require("@self/inline_block").InlineBlock,
	InlineComposer = require("@self/inline_block").Composer,
	InlineThread = require("@self/inline_block").Thread,
	InlineNote = require("@self/inline_block").Note,
	InlineAdd = require("@self/inline_block").Add,
	Wallpaper = require("@self/wallpaper").Wallpaper,
	wallpaper = require("@self/wallpaper"),
	FilesPane = require("@self/files_pane"),
	Onboarding = onboarding_frame,
	Status = status.Status,
	StatusDot = status.StatusDot,
	StatusIcon = status.StatusIcon,
	StatusTag = status.StatusTag,
	material = require("@self/material"),
}
local index = setmetatable(ds, {
	__index = function(t: any, key: string): any
		local load = lazy[key]
		if not load then
			return nil
		end
		local v = load()
		rawset(t, key, v)
		return v
	end,
})

-- (Warmed a few a turn from a timer once the launch is drawn, so a first use seldom compiles one in its own call.)
if ui ~= nil and ui.after ~= nil then
	local order: { string } = {}
	for key in lazy do
		table.insert(order, key)
	end
	table.sort(order)
	local function warm(from: number)
		local started, i = ui.now(), from
		while i <= #order and ui.now() - started < 8 do
			local _ = index[order[i]]
			i += 1
		end
		if i <= #order then
			ui.after(16, function()
				warm(i)
			end)
		end
	end
	ui.after(0, function()
		warm(1)
	end)
end

return (index :: any) :: typeof(all())
