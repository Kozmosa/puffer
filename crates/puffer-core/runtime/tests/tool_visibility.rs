use super::*;
use puffer_resources::load_resources;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use time::{format_description, OffsetDateTime};

const NOTEBOOK_EDIT_DESCRIPTION: &str =
    "Completely replaces the contents of a specific cell in a Jupyter notebook\n(.ipynb file) with new source.\n\nJupyter notebooks combine code, text, and visualizations for data analysis\nand scientific computing.\n\nUsage:\n- `notebook_path` must be an absolute path.\n- Read the notebook with `Read` before editing it. This tool fails if the\n  notebook has not been fully read or if it changed after it was read.\n- `cell_id` identifies the target cell. Existing cell ids and `cell-N` or\n  numeric index fallbacks are accepted.\n- Use `edit_mode: \"insert\"` to add a new cell after `cell_id`, or at the\n  beginning if `cell_id` is omitted.\n- Use `edit_mode: \"delete\"` to remove the target cell.\n- `cell_type` is required when inserting and may be `code` or `markdown`.";
const CONFIG_TOOL_SNIPPETS: &[&str] = &[
    "Get or set Claude Code configuration settings.",
    "### Global Settings",
    "copy_full_response",
    "### Project Settings",
    "openai_headers",
    "### Session Settings",
    "statuslineEnabled",
];
const AGENT_TOOL_SNIPPETS: &[&str] = &[
    "Launch a new agent to handle complex, multi-step tasks autonomously.",
    "Available agent types and the tools they have access to:",
    "- general-purpose:",
    "- Explore:",
    "Launch multiple agents concurrently whenever possible",
    "To continue a previously spawned agent, use SendMessage",
    "`isolation: \"worktree\"`",
    "## Writing the prompt",
    "Never delegate understanding.",
];
const ASK_USER_QUESTION_DESCRIPTION: &str = "Use this tool when you need to ask the user questions during execution. This allows you to:\n1. Gather user preferences or requirements\n2. Clarify ambiguous instructions\n3. Get decisions on implementation choices as you work\n4. Offer choices to the user about what direction to take.\n\nUsage notes:\n- Users will always be able to select \"Other\" to provide custom text input\n- Use multiSelect: true to allow multiple answers to be selected for a question\n- If you recommend a specific option, make that the first option in the list and add \"(Recommended)\" at the end of the label\n\nPlan mode note: In plan mode, use this tool to clarify requirements or choose between approaches BEFORE finalizing your plan. Do NOT use this tool to ask \"Is my plan ready?\" or \"Should I proceed?\" - use ExitPlanMode for plan approval. IMPORTANT: Do not reference \"the plan\" in your questions (e.g., \"Do you have feedback about the plan?\", \"Does the plan look good?\") because the user cannot see the plan in the UI until you call ExitPlanMode. If you need plan approval, use ExitPlanMode instead.\nPreview feature:\nUse the optional `preview` field on options when presenting concrete artifacts that users need to visually compare:\n- ASCII mockups of UI layouts or components\n- Code snippets showing different implementations\n- Diagram variations\n- Configuration examples\n\nPreview content is rendered as markdown in a monospace box. Multi-line text with newlines is supported. When any option has a preview, the UI switches to a side-by-side layout with a vertical option list on the left and preview on the right. Do not use previews for simple preference questions where labels and descriptions suffice. Note: previews are only supported for single-select questions (not multiSelect).";
const ENTER_PLAN_MODE_DESCRIPTION: &str = "Use this tool proactively when you're about to start a non-trivial implementation task. Getting user sign-off on your approach before writing code prevents wasted effort and ensures alignment. This tool transitions you into plan mode where you can explore the codebase and design an implementation approach for user approval.\n\n## When to Use This Tool\n\n**Prefer using EnterPlanMode** for implementation tasks unless they're simple. Use it when ANY of these conditions apply:\n\n1. **New Feature Implementation**: Adding meaningful new functionality\n   - Example: \"Add a logout button\" - where should it go? What should happen on click?\n   - Example: \"Add form validation\" - what rules? What error messages?\n\n2. **Multiple Valid Approaches**: The task can be solved in several different ways\n   - Example: \"Add caching to the API\" - could use Redis, in-memory, file-based, etc.\n   - Example: \"Improve performance\" - many optimization strategies possible\n\n3. **Code Modifications**: Changes that affect existing behavior or structure\n   - Example: \"Update the login flow\" - what exactly should change?\n   - Example: \"Refactor this component\" - what's the target architecture?\n\n4. **Architectural Decisions**: The task requires choosing between patterns or technologies\n   - Example: \"Add real-time updates\" - WebSockets vs SSE vs polling\n   - Example: \"Implement state management\" - Redux vs Context vs custom solution\n\n5. **Multi-File Changes**: The task will likely touch more than 2-3 files\n   - Example: \"Refactor the authentication system\"\n   - Example: \"Add a new API endpoint with tests\"\n\n6. **Unclear Requirements**: You need to explore before understanding the full scope\n   - Example: \"Make the app faster\" - need to profile and identify bottlenecks\n   - Example: \"Fix the bug in checkout\" - need to investigate root cause\n\n7. **User Preferences Matter**: The implementation could reasonably go multiple ways\n   - If you would use AskUserQuestion to clarify the approach, use EnterPlanMode instead\n   - Plan mode lets you explore first, then present options with context\n\n## When NOT to Use This Tool\n\nOnly skip EnterPlanMode for simple tasks:\n- Single-line or few-line fixes (typos, obvious bugs, small tweaks)\n- Adding a single function with clear requirements\n- Tasks where the user has given very specific, detailed instructions\n- Pure research/exploration tasks (use the Agent tool with explore agent instead)\n\n## What Happens in Plan Mode\n\nIn plan mode, you'll:\n1. Thoroughly explore the codebase using Glob, Grep, and Read tools\n2. Understand existing patterns and architecture\n3. Design an implementation approach\n4. Present your plan to the user for approval\n5. Use AskUserQuestion if you need to clarify approaches\n6. Exit plan mode with ExitPlanMode when ready to implement\n\n## Examples\n\n### GOOD - Use EnterPlanMode:\nUser: \"Add user authentication to the app\"\n- Requires architectural decisions (session vs JWT, where to store tokens, middleware structure)\n\nUser: \"Optimize the database queries\"\n- Multiple approaches possible, need to profile first, significant impact\n\nUser: \"Implement dark mode\"\n- Architectural decision on theme system, affects many components\n\nUser: \"Add a delete button to the user profile\"\n- Seems simple but involves: where to place it, confirmation dialog, API call, error handling, state updates\n\nUser: \"Update the error handling in the API\"\n- Affects multiple files, user should approve the approach\n\n### BAD - Don't use EnterPlanMode:\nUser: \"Fix the typo in the README\"\n- Straightforward, no planning needed\n\nUser: \"Add a console.log to debug this function\"\n- Simple, obvious implementation\n\nUser: \"What files handle routing?\"\n- Research task, not implementation planning\n\n## Important Notes\n\n- This tool REQUIRES user approval - they must consent to entering plan mode\n- If unsure whether to use it, err on the side of planning - it's better to get alignment upfront than to redo work\n- Users appreciate being consulted before significant changes are made to their codebase";
const EXIT_PLAN_MODE_DESCRIPTION: &str = "Use this tool when you are in plan mode and have finished writing your plan to the plan file and are ready for user approval.\n\n## How This Tool Works\n- You should have already written your plan to the plan file specified in the plan mode system message\n- This tool does NOT take the plan content as a parameter - it will read the plan from the file you wrote\n- This tool simply signals that you're done planning and ready for the user to review and approve\n- The user will see the contents of your plan file when they review it\n\n## When to Use This Tool\nIMPORTANT: Only use this tool when the task requires planning the implementation steps of a task that requires writing code. For research tasks where you're gathering information, searching files, reading files or in general trying to understand the codebase - do NOT use this tool.\n\n## Before Using This Tool\nEnsure your plan is complete and unambiguous:\n- If you have unresolved questions about requirements or approach, use AskUserQuestion first (in earlier phases)\n- Once your plan is finalized, use THIS tool to request approval\n\n**Important:** Do NOT use AskUserQuestion to ask \"Is this plan okay?\" or \"Should I proceed?\" - that's exactly what THIS tool does. ExitPlanMode inherently requests user approval of your plan.\n\n## Examples\n\n1. Initial task: \"Search for and understand the implementation of vim mode in the codebase\" - Do not use the exit plan mode tool because you are not planning the implementation steps of a task.\n2. Initial task: \"Help me implement yank mode for vim\" - Use the exit plan mode tool after you have finished planning the implementation steps of the task.\n3. Initial task: \"Add a new feature to handle user authentication\" - If unsure about auth method (OAuth, JWT, etc.), use AskUserQuestion first, then use exit plan mode tool after clarifying the approach.";
const TODO_WRITE_DESCRIPTION: &str = "Use this tool to create and manage a structured task list for your current coding session. This helps you track progress, organize complex tasks, and demonstrate thoroughness to the user.\nIt also helps the user understand the progress of the task and overall progress of their requests.\n\n## When to Use This Tool\nUse this tool proactively in these scenarios:\n\n1. Complex multi-step tasks - When a task requires 3 or more distinct steps or actions\n2. Non-trivial and complex tasks - Tasks that require careful planning or multiple operations\n3. User explicitly requests todo list - When the user directly asks you to use the todo list\n4. User provides multiple tasks - When users provide a list of things to be done (numbered or comma-separated)\n5. After receiving new instructions - Immediately capture user requirements as todos\n6. When you start working on a task - Mark it as in_progress BEFORE beginning work. Ideally you should only have one todo as in_progress at a time\n7. After completing a task - Mark it as completed and add any new follow-up tasks discovered during implementation\n\n## When NOT to Use This Tool\n\nSkip using this tool when:\n1. There is only a single, straightforward task\n2. The task is trivial and tracking it provides no organizational benefit\n3. The task can be completed in less than 3 trivial steps\n4. The task is purely conversational or informational\n\nNOTE that you should not use this tool if there is only one trivial task to do. In this case you are better off just doing the task directly.\n\n## Examples of When to Use the Todo List\n\n<example>\nUser: I want to add a dark mode toggle to the application settings. Make sure you run the tests and build when you're done!\nAssistant: *Creates todo list with the following items:*\n1. Creating dark mode toggle component in Settings page\n2. Adding dark mode state management (context/store)\n3. Implementing CSS-in-JS styles for dark theme\n4. Updating existing components to support theme switching\n5. Running tests and build process, addressing any failures or errors that occur\n*Begins working on the first task*\n\n<reasoning>\nThe assistant used the todo list because:\n1. Adding dark mode is a multi-step feature requiring UI, state management, and styling changes\n2. The user explicitly requested tests and build be run afterward\n3. The assistant inferred that tests and build need to pass by adding \"Ensure tests and build succeed\" as the final task\n</reasoning>\n</example>\n\n<example>\nUser: Help me rename the function getCwd to getCurrentWorkingDirectory across my project\nAssistant: *Uses grep or search tools to locate all instances of getCwd in the codebase*\nI've found 15 instances of 'getCwd' across 8 different files.\n*Creates todo list with specific items for each file that needs updating*\n\n<reasoning>\nThe assistant used the todo list because:\n1. First, the assistant searched to understand the scope of the task\n2. Upon finding multiple occurrences across different files, it determined this was a complex task with multiple steps\n3. The todo list helps ensure every instance is tracked and updated systematically\n4. This approach prevents missing any occurrences and maintains code consistency\n</reasoning>\n</example>\n\n\n<example>\nUser: I need to implement these features for my e-commerce site: user registration, product catalog, shopping cart, and checkout flow.\nAssistant: *Creates a todo list breaking down each feature into specific tasks based on the project architecture*\nLet's start with user registration. This will involve creating database models, API endpoints, and frontend forms that integrate with your existing authentication system.\n\n<reasoning>\nThe assistant used the todo list because:\n1. The user provided multiple complex features to implement in a comma separated list\n2. The todo list helps organize these large features into manageable tasks\n3. This approach allows for tracking progress across the entire implementation\n</reasoning>\n</example>\n\n<example>\nUser: Can you help optimize my React application? It's rendering slowly and has performance issues.\nAssistant: *Reviews component structure, render patterns, state management, and data fetching*\nAfter analyzing your codebase, I've identified several performance issues.\n*Creates todo list with items like: 1) Implementing memoization for expensive calculations in ProductList, 2) Adding virtualization for long lists in Dashboard, 3) Optimizing image loading in Gallery component, 4) Fixing state update loops in ShoppingCart, 5) Reviewing bundle size and implementing code splitting*\nLet's start by implementing memoization for the expensive calculations in your ProductList component.\n\n<reasoning>\nThe assistant used the todo list because:\n1. First, the assistant examined the codebase to identify specific performance issues\n2. Based on this analysis, it identified multiple optimization opportunities\n3. Performance optimization is a non-trivial task requiring multiple steps\n4. The todo list helps methodically track improvements across different components\n5. This systematic approach ensures all performance bottlenecks are addressed\n</reasoning>\n</example>\n\n## Examples of When NOT to Use the Todo List\n\n<example>\nUser: How do I print 'Hello World' in Python?\nAssistant: In Python, you can print \"Hello World\" with this simple code:\n\npython\nprint(\"Hello World\")\n\nThis will output the text \"Hello World\" to the console when executed.\n\n<reasoning>\nThe assistant did not use the todo list because this is a single, trivial task that can be completed in one step. There's no need to track multiple tasks or steps for such a straightforward request.\n</reasoning>\n</example>\n\n<example>\nUser: What does the git status command do?\nAssistant: The git status command shows the current state of your working directory and staging area. It displays which changes have been staged, which haven't, and which files aren't being tracked by Git. This is a fundamental command for understanding the current state of your repository before making commits.\n\n<reasoning>\nThe assistant did not use the todo list because this is an informational request with no actual coding task to complete. The user is simply asking for an explanation, not for the assistant to perform multiple steps or tasks.\n</reasoning>\n</example>\n\n<example>\nUser: Can you add a comment to the calculateTotal function to explain what it does?\nAssistant: Sure, let me add a comment to the calculateTotal function to explain what it does.\n* Uses the Edit tool to add a comment to the calculateTotal function *\n\n<reasoning>\nThe assistant did not use the todo list because this is a single, straightforward task confined to one location in the code. Adding a comment doesn't require tracking multiple steps or systematic organization.\n</reasoning>\n</example>\n\n<example>\nUser: Run npm install for me and tell me what happens.\nAssistant: I'll run the npm install command for you.\n\n*Executes: npm install*\n\nThe command completed successfully. Here's the output:\n[Output of npm install command]\n\nAll dependencies have been installed according to your package.json file.\n\n<reasoning>\nThe assistant did not use the todo list because this is a single command execution with immediate results. There are no multiple steps to track or organize, making the todo list unnecessary for this straightforward task.\n</reasoning>\n</example>\n\n## Task States and Management\n\n1. **Task States**: Use these states to track progress:\n   - pending: Task not yet started\n   - in_progress: Currently working on (limit to ONE task at a time)\n   - completed: Task finished successfully\n\n   **IMPORTANT**: Task descriptions must have two forms:\n   - content: The imperative form describing what needs to be done (e.g., \"Run tests\", \"Build the project\")\n   - activeForm: The present continuous form shown during execution (e.g., \"Running tests\", \"Building the project\")\n\n2. **Task Management**:\n   - Update task status in real-time as you work\n   - Mark tasks complete IMMEDIATELY after finishing (don't batch completions)\n   - Exactly ONE task must be in_progress at any time (not less, not more)\n   - Complete current tasks before starting new ones\n   - Remove tasks that are no longer relevant from the list entirely\n\n3. **Task Completion Requirements**:\n   - ONLY mark a task as completed when you have FULLY accomplished it\n   - If you encounter errors, blockers, or cannot finish, keep the task as in_progress\n   - When blocked, create a new task describing what needs to be resolved\n   - Never mark a task as completed if:\n     - Tests are failing\n     - Implementation is partial\n     - You encountered unresolved errors\n     - You couldn't find necessary files or dependencies\n\n4. **Task Breakdown**:\n   - Create specific, actionable items\n   - Break complex tasks into smaller, manageable steps\n   - Use clear, descriptive task names\n   - Always provide both forms:\n     - content: \"Fix authentication bug\"\n     - activeForm: \"Fixing authentication bug\"\n\nWhen in doubt, use this tool. Being proactive with task management demonstrates attentiveness and ensures you complete all requirements successfully.";

#[test]
fn sleep_tool_is_visible_to_anthropic_and_openai_tool_builders() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let expected = reference_sleep_prompt();

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_sleep = anthropic
        .iter()
        .find(|definition| definition["name"] == json!("Sleep"))
        .expect("Sleep tool definition");
    assert_eq!(anthropic_sleep["description"], json!(expected.clone()));
    assert_eq!(
        anthropic_sleep["input_schema"]["required"],
        json!(["duration_ms"])
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_sleep = openai
        .iter()
        .find(|definition| definition.name == "Sleep")
        .expect("Sleep tool definition");
    assert_eq!(openai_sleep.description, expected);
    assert_eq!(openai_sleep.parameters["required"], json!(["duration_ms"]));
}

#[test]
fn bundled_resources_register_sleep_tool() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let definition = registry.definition("Sleep").expect("Sleep tool definition");

    assert_eq!(definition.handler, "runtime:sleep");
    assert_eq!(definition.description, reference_sleep_prompt());
}

#[test]
fn notebook_edit_tool_is_visible_to_anthropic_and_openai_tool_builders() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_notebook = anthropic
        .iter()
        .find(|definition| definition["name"] == json!("NotebookEdit"))
        .expect("NotebookEdit tool definition");
    assert_eq!(
        anthropic_notebook["description"],
        json!(NOTEBOOK_EDIT_DESCRIPTION)
    );
    assert_eq!(
        anthropic_notebook["input_schema"]["required"],
        json!(["notebook_path", "new_source"])
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_notebook = openai
        .iter()
        .find(|definition| definition.name == "NotebookEdit")
        .expect("NotebookEdit tool definition");
    assert_eq!(openai_notebook.description, NOTEBOOK_EDIT_DESCRIPTION);
    assert_eq!(
        openai_notebook.parameters["required"],
        json!(["notebook_path", "new_source"])
    );
}

#[test]
fn bundled_resources_register_notebook_edit_tool() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let definition = registry
        .definition("NotebookEdit")
        .expect("NotebookEdit tool definition");

    assert_eq!(definition.handler, "runtime:notebook_edit");
    assert_eq!(definition.description, NOTEBOOK_EDIT_DESCRIPTION);
    assert_eq!(definition.display.group.as_deref(), Some("files"));
    assert_eq!(definition.display.title.as_deref(), Some("NotebookEdit"));
    assert!(definition.display.show_in_status);
}

#[test]
fn workflow_tool_descriptions_match_claude_reference_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);

    for tool_id in [
        "AskUserQuestion",
        "EnterPlanMode",
        "ExitPlanMode",
        "Skill",
        "TodoWrite",
        "ToolSearch",
        "SendMessage",
        "SendUserMessage",
        "TaskGet",
        "TaskList",
        "TaskStop",
        "TaskOutput",
    ] {
        let description = registry
            .definition(tool_id)
            .expect("tool definition")
            .description
            .clone();

        let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
        let anthropic_definition = anthropic
            .iter()
            .find(|item| item["name"] == json!(tool_id))
            .expect("anthropic tool definition");
        assert_eq!(
            anthropic_definition["description"],
            json!(description.clone())
        );

        let openai = openai_tool_definitions(&registry, None, false).unwrap();
        let openai_definition = openai
            .iter()
            .find(|item| item.name == tool_id)
            .expect("openai tool definition");
        assert_eq!(openai_definition.description, description);
    }
}

#[test]
fn agent_tool_description_is_rendered_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let description = registry
        .definition("Agent")
        .expect("Agent tool definition")
        .description
        .clone();

    for snippet in AGENT_TOOL_SNIPPETS {
        assert!(
            description.contains(snippet),
            "Agent description missing snippet: {snippet}"
        );
    }

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_definition = anthropic
        .iter()
        .find(|item| item["name"] == json!("Agent"))
        .expect("anthropic Agent tool definition");
    assert_eq!(
        anthropic_definition["description"],
        json!(description.clone())
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_definition = openai
        .iter()
        .find(|item| item.name == "Agent")
        .expect("openai Agent tool definition");
    assert_eq!(openai_definition.description, description);
}

#[test]
fn config_tool_description_is_rendered_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let description = registry
        .definition("Config")
        .expect("Config tool definition")
        .description
        .clone();

    assert!(description.contains("### Global Settings"));
    assert!(description.contains("copy_full_response"));
    assert!(description.contains("### Project Settings"));
    assert!(description.contains("openai_headers"));
    assert!(description.contains("### Session Settings"));
    assert!(description.contains("statuslineEnabled"));

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_definition = anthropic
        .iter()
        .find(|item| item["name"] == json!("Config"))
        .expect("anthropic Config tool definition");
    assert_eq!(
        anthropic_definition["description"],
        json!(description.clone())
    );
    assert!(
        anthropic_definition["input_schema"]["properties"]["value"]["oneOf"]
            .as_array()
            .is_some_and(|variants| variants.iter().any(|variant| variant["type"] == "integer"))
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_definition = openai
        .iter()
        .find(|item| item.name == "Config")
        .expect("openai Config tool definition");
    assert_eq!(openai_definition.description, description);
    assert!(openai_definition.parameters["properties"]["value"]["oneOf"]
        .as_array()
        .is_some_and(|variants| variants.iter().any(|variant| variant["type"] == "integer")));
}

#[test]
fn lsp_tool_description_matches_claude_reference_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    assert_tool_description_matches_expected(&registry, "LSP", reference_lsp_prompt().as_str());
}

#[test]
fn powershell_tool_description_matches_claude_reference_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let expected = reference_powershell_prompt();
    let description = registry
        .definition("PowerShell")
        .expect("PowerShell tool definition")
        .description
        .clone();
    assert_eq!(
        normalize_prompt_lines(&description),
        normalize_prompt_lines(&expected)
    );

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_definition = anthropic
        .iter()
        .find(|definition| definition["name"] == json!("PowerShell"))
        .expect("PowerShell anthropic tool definition");
    assert_eq!(
        normalize_prompt_lines(
            anthropic_definition["description"]
                .as_str()
                .expect("anthropic description"),
        ),
        normalize_prompt_lines(&expected)
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_definition = openai
        .iter()
        .find(|definition| definition.name == "PowerShell")
        .expect("PowerShell openai tool definition");
    assert_eq!(
        normalize_prompt_lines(&openai_definition.description),
        normalize_prompt_lines(&expected)
    );
    assert_eq!(
        openai_definition.parameters["properties"]["timeout"]["description"],
        json!("Optional timeout in milliseconds (max 600000)")
    );
    assert_eq!(
        openai_definition.parameters["properties"]["run_in_background"]["description"],
        json!(
            "Set to true to run this command in the background. Use Read to read the output later."
        )
    );
    assert_eq!(
        openai_definition.parameters["properties"]["dangerouslyDisableSandbox"]["description"],
        json!(
            "Set this to true to dangerously override sandbox mode and run commands without sandboxing."
        )
    );
}

#[test]
fn web_search_tool_prompt_matches_claude_reference_for_anthropic_and_openai() {
    let reference = read_repo_file("references/claude-code/src/tools/WebSearchTool/prompt.ts");
    let expected =
        normalize_reference_template(&extract_template_literal(&reference, "  return `"))
            .replace("${currentMonthYear}", &current_month_year());
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_definition = anthropic
        .iter()
        .find(|definition| definition["name"] == json!("WebSearch"))
        .expect("WebSearch anthropic tool definition");
    assert_eq!(anthropic_definition["description"], json!(expected.clone()));

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_definition = openai
        .iter()
        .find(|definition| definition.name == "WebSearch")
        .expect("WebSearch openai tool definition");
    assert_eq!(openai_definition.description, expected);
}

#[test]
fn selected_tool_prompts_match_claude_reference_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let openai = openai_tool_definitions(&registry, None, false).unwrap();

    for (tool_id, expected) in [
        ("TaskCreate", reference_task_create_prompt()),
        ("TaskUpdate", reference_task_update_prompt()),
        ("TeamCreate", reference_team_create_prompt()),
        ("TeamDelete", reference_team_delete_prompt()),
        ("EnterWorktree", reference_enter_worktree_prompt()),
        ("ExitWorktree", reference_exit_worktree_prompt()),
        (
            "ListMcpResourcesTool",
            reference_list_mcp_resources_prompt(),
        ),
        ("ReadMcpResourceTool", reference_read_mcp_resource_prompt()),
        ("ToolSearch", reference_tool_search_prompt()),
        ("WebFetch", reference_web_fetch_prompt()),
    ] {
        let definition = registry.definition(tool_id).expect("tool definition");
        assert_eq!(
            definition.description.trim_end(),
            expected.trim_end(),
            "registry description for {tool_id}"
        );

        let anthropic_definition = anthropic
            .iter()
            .find(|item| item["name"] == json!(tool_id))
            .expect("anthropic tool definition");
        assert_eq!(
            anthropic_definition["description"]
                .as_str()
                .expect("anthropic description")
                .trim_end(),
            expected.trim_end(),
            "anthropic description for {tool_id}"
        );

        let openai_definition = openai
            .iter()
            .find(|item| item.name == tool_id)
            .expect("openai tool definition");
        assert_eq!(
            openai_definition.description.trim_end(),
            expected.trim_end(),
            "openai description for {tool_id}"
        );
    }
}

#[test]
fn built_in_agent_resources_match_claude_reference_prompts() {
    let resources = bundled_resources();

    for (agent_id, expected_description, expected_prompt) in [
        (
            "Explore",
            reference_explore_agent_description(),
            reference_explore_agent_prompt(),
        ),
        (
            "general-purpose",
            reference_general_purpose_agent_description(),
            reference_general_purpose_agent_prompt(),
        ),
        (
            "Plan",
            reference_plan_agent_description(),
            reference_plan_agent_prompt(),
        ),
        (
            "statusline-setup",
            reference_statusline_setup_agent_description(),
            reference_statusline_setup_agent_prompt(),
        ),
        (
            "verification",
            reference_verification_agent_description(),
            reference_verification_agent_prompt(),
        ),
    ] {
        let agent = resources
            .agents
            .iter()
            .find(|item| item.value.id == agent_id)
            .unwrap_or_else(|| panic!("missing builtin agent {agent_id}"));

        assert_eq!(
            normalize_inline_whitespace(&agent.value.description),
            normalize_inline_whitespace(&expected_description),
            "description for {agent_id}"
        );
        assert_eq!(
            normalize_agent_prompt_text(&agent.value.prompt),
            normalize_agent_prompt_text(&expected_prompt),
            "prompt for {agent_id}"
        );
    }
}

#[test]
fn file_tool_prompts_and_schemas_match_claude_reference_for_anthropic_and_openai() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let openai = openai_tool_definitions(&registry, None, false).unwrap();

    for (tool_id, expected_prompt, expected_schema) in [
        (
            "Read",
            reference_file_read_prompt(),
            reference_file_read_schema(),
        ),
        (
            "Edit",
            reference_file_edit_prompt(),
            reference_file_edit_schema(),
        ),
        (
            "Write",
            reference_file_write_prompt(),
            reference_file_write_schema(),
        ),
    ] {
        let definition = registry.definition(tool_id).expect("tool definition");
        assert_eq!(
            definition.description.trim_end(),
            expected_prompt.trim_end(),
            "registry description for {tool_id}"
        );
        assert_eq!(
            definition.input_schema.as_json_schema(),
            expected_schema,
            "registry schema for {tool_id}"
        );

        let anthropic_definition = anthropic
            .iter()
            .find(|item| item["name"] == json!(tool_id))
            .expect("anthropic tool definition");
        assert_eq!(
            anthropic_definition["description"]
                .as_str()
                .expect("anthropic description")
                .trim_end(),
            expected_prompt.trim_end(),
            "anthropic description for {tool_id}"
        );
        assert_eq!(
            anthropic_definition["input_schema"], expected_schema,
            "anthropic schema for {tool_id}"
        );

        let openai_definition = openai
            .iter()
            .find(|item| item.name == tool_id)
            .expect("openai tool definition");
        assert_eq!(
            openai_definition.description.trim_end(),
            expected_prompt.trim_end(),
            "openai description for {tool_id}"
        );
        assert_eq!(
            openai_definition.parameters, expected_schema,
            "openai schema for {tool_id}"
        );
    }
}

#[test]
fn ask_user_question_schema_supports_multi_select_answers_for_both_providers() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let expected = json!([
        { "type": "string" },
        {
            "type": "array",
            "items": { "type": "string" }
        }
    ]);

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_definition = anthropic
        .iter()
        .find(|item| item["name"] == json!("AskUserQuestion"))
        .expect("anthropic AskUserQuestion tool definition");
    assert_eq!(
        anthropic_definition["input_schema"]["properties"]["answers"]["additionalProperties"]
            ["oneOf"],
        expected
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_definition = openai
        .iter()
        .find(|item| item.name == "AskUserQuestion")
        .expect("openai AskUserQuestion tool definition");
    assert_eq!(
        openai_definition.parameters["properties"]["answers"]["additionalProperties"]["oneOf"],
        expected
    );
}

fn bundled_resources() -> LoadedResources {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths {
        workspace_root: temp.path().join("workspace"),
        workspace_config_dir: temp.path().join("workspace/.puffer"),
        user_config_dir: temp.path().join("user"),
        builtin_resources_dir: root.join("resources"),
    };
    load_resources(&paths).unwrap()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(workspace_root().join(relative_path)).unwrap()
}

fn extract_template_literal(contents: &str, marker: &str) -> String {
    let start = contents.find(marker).unwrap() + marker.len();
    let source = &contents[start..];
    let mut end = None;
    let mut index = 0usize;
    let mut escaped = false;
    let mut interpolation_depth = 0usize;

    while index < source.len() {
        let ch = source[index..].chars().next().unwrap();
        let width = ch.len_utf8();
        if escaped {
            escaped = false;
            index += width;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += width;
            continue;
        }
        if interpolation_depth == 0 && ch == '`' {
            end = Some(start + index);
            break;
        }
        if source[index..].starts_with("${") {
            interpolation_depth += 1;
            index += 2;
            continue;
        }
        if interpolation_depth > 0 {
            match ch {
                '{' => interpolation_depth += 1,
                '}' => interpolation_depth = interpolation_depth.saturating_sub(1),
                _ => {}
            }
        }
        index += width;
    }

    contents[start..end.unwrap()].to_string()
}

fn normalize_reference_template(raw: &str) -> String {
    let unescaped = raw.replace("\\`", "`");
    let trimmed = unescaped.strip_prefix('\n').unwrap_or(&unescaped);
    dedent(trimmed)
}

fn decode_js_template_escapes(raw: &str) -> String {
    let mut output = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => output.push('\\'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('"') => output.push('"'),
            Some('`') => output.push('`'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }
    output
}

fn trim_line_trailing_whitespace(raw: &str) -> String {
    raw.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_agent_prompt_text(raw: &str) -> String {
    normalize_inline_whitespace(
        &trim_line_trailing_whitespace(&decode_js_template_escapes(raw)).replace(
            "file creation or modification",
            "file creation/modification",
        ),
    )
    .replace(['—', '–'], "-")
    .replace(['“', '”'], "\"")
    .replace(['’', '‘'], "'")
}

fn dedent(raw: &str) -> String {
    let indent = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|ch| *ch == ' ').count())
        .min()
        .unwrap_or(0);
    raw.lines()
        .map(|line| line.strip_prefix(&" ".repeat(indent)).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn current_month_year() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let format =
        format_description::parse("[month repr:long] [year]").expect("valid month/year format");
    now.format(&format).unwrap()
}

fn assert_tool_description_matches_expected(
    registry: &ToolRegistry,
    tool_id: &str,
    expected: &str,
) {
    let expected = normalize_tool_description(tool_id, expected);
    let description = registry
        .definition(tool_id)
        .expect("tool definition")
        .description
        .clone();
    assert_eq!(normalize_tool_description(tool_id, &description), expected);

    let anthropic = anthropic_tool_definitions(registry, None).unwrap();
    let anthropic_definition = anthropic
        .iter()
        .find(|item| item["name"] == json!(tool_id))
        .expect("anthropic tool definition");
    assert_eq!(
        normalize_tool_description(
            tool_id,
            anthropic_definition["description"]
                .as_str()
                .expect("anthropic description"),
        ),
        expected,
        "anthropic description for {tool_id}"
    );

    let openai = openai_tool_definitions(registry, None, false).unwrap();
    let openai_definition = openai
        .iter()
        .find(|item| item.name == tool_id)
        .expect("openai tool definition");
    assert_eq!(
        normalize_tool_description(tool_id, &openai_definition.description),
        expected,
        "openai description for {tool_id}"
    );
}

fn normalize_tool_description(tool_id: &str, raw: &str) -> String {
    let trimmed = trim_line_trailing_whitespace(raw);
    if tool_id == "PowerShell" {
        return trimmed
            .replace(
                "\n- You can use the `run_in_background` parameter",
                "\n  - You can use the `run_in_background` parameter",
            )
            .replace(
                "\n\n  - Avoid using PowerShell to run commands that have dedicated tools, unless explicitly instructed:",
                "\n  - Avoid using PowerShell to run commands that have dedicated tools, unless explicitly instructed:",
            )
            .replace(
                "\n\n  - For git commands:",
                "\n  - For git commands:",
            );
    }
    trimmed
}

fn reference_sleep_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/SleepTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(
        &reference,
        "export const SLEEP_TOOL_PROMPT = `",
    ))
    .replace("${TICK_TAG}", "tick")
}

fn reference_lsp_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/LSPTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(
        &reference,
        "export const DESCRIPTION = `",
    ))
}

fn reference_powershell_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/PowerShellTool/prompt.ts");
    let prompt = normalize_reference_template(&extract_template_literal(
        reference
            .split("export async function getPrompt(): Promise<string> {")
            .nth(1)
            .expect("PowerShell prompt section"),
        "  return `",
    ));
    let background = normalize_reference_template(&extract_template_literal(
        reference
            .split("function getBackgroundUsageNote(): string | null {")
            .nth(1)
            .expect("PowerShell background section"),
        "  return `",
    ));
    let sleep = normalize_reference_template(&extract_template_literal(
        reference
            .split("function getSleepGuidance(): string | null {")
            .nth(1)
            .expect("PowerShell sleep section"),
        "  return `",
    ));
    let edition = normalize_reference_template(&extract_template_literal(
        reference
            .split("// Detection not yet resolved (first prompt build before any tool call) or")
            .nth(1)
            .expect("PowerShell unknown edition section"),
        "  return `",
    ));
    decode_js_escapes(
        &prompt
            .replace("${getEditionSection(edition)}", &edition)
            .replace("${getMaxTimeoutMs()}", "600000")
            .replace("${getMaxTimeoutMs() / 60000}", "10")
            .replace("${getDefaultTimeoutMs()}", "120000")
            .replace("${getDefaultTimeoutMs() / 60000}", "2")
            .replace("${getMaxOutputLength()}", "30000")
            .replace(
                "${backgroundNote ? backgroundNote + '\\n' : ''}\\",
                &(indent_block(&background, 2) + "\n"),
            )
            .replace(
                "${sleepGuidance ? sleepGuidance + '\\n' : ''}\\",
                &(indent_block(&sleep, 2) + "\n\n"),
            )
            .replace("${GLOB_TOOL_NAME}", "Glob")
            .replace("${GREP_TOOL_NAME}", "Grep")
            .replace("${FILE_READ_TOOL_NAME}", "Read")
            .replace("${FILE_EDIT_TOOL_NAME}", "Edit")
            .replace("${FILE_WRITE_TOOL_NAME}", "Write")
            .replace("${POWERSHELL_TOOL_NAME}", "PowerShell")
            .replace("\n\n\n  - For git commands:", "\n\n  - For git commands:"),
    )
}

fn reference_task_create_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/TaskCreateTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
        .replace("${teammateContext}", " and potentially assigned to teammates")
        .replace(
            "${teammateTips}",
            "- Include enough detail in the description for another agent to understand and complete the task\n- New tasks are created with status 'pending' and no owner - use TaskUpdate with the `owner` parameter to assign them\n",
        )
}

fn reference_task_update_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/TaskUpdateTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(
        &reference,
        "export const PROMPT = `",
    ))
}

fn reference_team_create_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/TeamCreateTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
}

fn reference_team_delete_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/TeamDeleteTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
}

fn reference_enter_worktree_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/EnterWorktreeTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
}

fn reference_exit_worktree_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/ExitWorktreeTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
}

fn reference_list_mcp_resources_prompt() -> String {
    let reference =
        read_repo_file("references/claude-code/src/tools/ListMcpResourcesTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(
        &reference,
        "export const PROMPT = `",
    ))
}

fn reference_read_mcp_resource_prompt() -> String {
    let reference =
        read_repo_file("references/claude-code/src/tools/ReadMcpResourceTool/prompt.ts");
    normalize_reference_template(&extract_template_literal(
        &reference,
        "export const PROMPT = `",
    ))
}

fn reference_tool_search_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/ToolSearchTool/prompt.ts");
    let head = normalize_reference_template(&extract_template_literal(
        &reference,
        "const PROMPT_HEAD = `",
    ));
    let tail = normalize_reference_template(&extract_template_literal(
        &reference,
        "const PROMPT_TAIL = `",
    ));
    format!("{head}Deferred tools appear by name in <system-reminder> messages.{tail}")
}

fn reference_web_fetch_prompt() -> String {
    let prompt_reference =
        read_repo_file("references/claude-code/src/tools/WebFetchTool/prompt.ts");
    let description = normalize_reference_template(&extract_template_literal(
        &prompt_reference,
        "export const DESCRIPTION = `",
    ));
    let tool_reference =
        read_repo_file("references/claude-code/src/tools/WebFetchTool/WebFetchTool.ts");
    let prompt_section = &tool_reference[tool_reference
        .find("  async prompt(_options) {")
        .expect("WebFetch prompt function")..];
    normalize_reference_template(&extract_template_literal(prompt_section, "    return `"))
        .replace("${DESCRIPTION}", &description)
}

fn reference_file_read_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/FileReadTool/prompt.ts");
    let template =
        normalize_reference_template(&extract_template_literal(&reference, "  return `"));
    let line_format =
        extract_quoted_literal(&reference, "export const LINE_FORMAT_INSTRUCTION =\n  ");
    let offset_instruction =
        extract_quoted_literal(&reference, "export const OFFSET_INSTRUCTION_DEFAULT =\n  ");
    let pdf_instruction = extract_quoted_literal(&reference, "      ? ");
    render_reference_template_literal(&template, |expr| match expr.trim() {
        "MAX_LINES_TO_READ" => "2000".to_string(),
        "maxSizeInstruction" => String::new(),
        "offsetInstruction" => offset_instruction.clone(),
        "lineFormat" => line_format.clone(),
        "BASH_TOOL_NAME" => "Bash".to_string(),
        expr if expr.contains("isPDFSupported()") => pdf_instruction.clone(),
        other => panic!("unexpected Read template expression: {other}"),
    })
}

fn reference_file_edit_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/FileEditTool/prompt.ts");
    let pre_read_section = reference
        .split("function getPreReadInstruction(): string {")
        .nth(1)
        .expect("Read pre-instruction section");
    let pre_read_template =
        normalize_reference_template(&extract_template_literal(pre_read_section, "  return `"));
    let pre_read =
        render_reference_template_literal(&pre_read_template, |expr| match expr.trim() {
            "FILE_READ_TOOL_NAME" => "Read".to_string(),
            other => panic!("unexpected Edit pre-read expression: {other}"),
        });

    let description_section = reference
        .split("function getDefaultEditDescription(): string {")
        .nth(1)
        .expect("Edit description section");
    let description_template =
        normalize_reference_template(&extract_template_literal(description_section, "  return `"));
    render_reference_template_literal(&description_template, |expr| match expr.trim() {
        "getPreReadInstruction()" => pre_read.clone(),
        "prefixFormat" => "line number + tab".to_string(),
        "minimalUniquenessHint" => String::new(),
        other => panic!("unexpected Edit template expression: {other}"),
    })
}

fn reference_file_write_prompt() -> String {
    let reference = read_repo_file("references/claude-code/src/tools/FileWriteTool/prompt.ts");
    let pre_read_section = reference
        .split("function getPreReadInstruction(): string {")
        .nth(1)
        .expect("Write pre-instruction section");
    let pre_read_template =
        normalize_reference_template(&extract_template_literal(pre_read_section, "  return `"));
    let pre_read =
        render_reference_template_literal(&pre_read_template, |expr| match expr.trim() {
            "FILE_READ_TOOL_NAME" => "Read".to_string(),
            other => panic!("unexpected Write pre-read expression: {other}"),
        });

    let description_section = reference
        .split("export function getWriteToolDescription(): string {")
        .nth(1)
        .expect("Write description section");
    let description_template =
        normalize_reference_template(&extract_template_literal(description_section, "  return `"));
    render_reference_template_literal(&description_template, |expr| match expr.trim() {
        "getPreReadInstruction()" => pre_read.clone(),
        other => panic!("unexpected Write template expression: {other}"),
    })
}

fn reference_file_read_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": "The absolute path to the file to read"
            },
            "offset": {
                "type": "integer",
                "description": "The line number to start reading from. Only provide if the file is too large to read at once",
                "minimum": 0
            },
            "limit": {
                "type": "integer",
                "description": "The number of lines to read. Only provide if the file is too large to read at once.",
                "minimum": 1
            },
            "pages": {
                "type": "string",
                "description": "Page range for PDF files (e.g., \"1-5\", \"3\", \"10-20\"). Only applicable to PDF files. Maximum 20 pages per request."
            }
        },
        "required": ["file_path"],
        "additionalProperties": false
    })
}

fn reference_file_edit_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": "The absolute path to the file to modify"
            },
            "old_string": {
                "type": "string",
                "description": "The text to replace"
            },
            "new_string": {
                "type": "string",
                "description": "The text to replace it with (must be different from old_string)"
            },
            "replace_all": {
                "type": "boolean",
                "description": "Replace all occurrences of old_string (default false)"
            }
        },
        "required": ["file_path", "old_string", "new_string"],
        "additionalProperties": false
    })
}

fn reference_file_write_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": "The absolute path to the file to write (must be absolute, not relative)"
            },
            "content": {
                "type": "string",
                "description": "The content to write to the file"
            }
        },
        "required": ["file_path", "content"],
        "additionalProperties": false
    })
}

fn extract_quoted_literal(contents: &str, marker: &str) -> String {
    let start = contents.find(marker).unwrap() + marker.len();
    let source = &contents[start..];
    let quote = source.chars().next().unwrap();
    assert!(quote == '\'' || quote == '"');
    let mut end = None;
    let mut index = quote.len_utf8();
    let mut escaped = false;

    while index < source.len() {
        let ch = source[index..].chars().next().unwrap();
        let width = ch.len_utf8();
        if escaped {
            escaped = false;
            index += width;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += width;
            continue;
        }
        if ch == quote {
            end = Some(index);
            break;
        }
        index += width;
    }

    decode_js_escapes(&source[quote.len_utf8()..end.unwrap()])
}

fn render_reference_template_literal(
    template: &str,
    mut replacer: impl FnMut(&str) -> String,
) -> String {
    let mut rendered = String::new();
    let mut index = 0usize;

    while index < template.len() {
        if template[index..].starts_with("${") {
            let expression_start = index + 2;
            let mut cursor = expression_start;
            let mut depth = 1usize;
            while cursor < template.len() {
                let ch = template[cursor..].chars().next().unwrap();
                let width = ch.len_utf8();
                if template[cursor..].starts_with("${") {
                    depth += 1;
                    cursor += 2;
                    continue;
                }
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            let expression = &template[expression_start..cursor];
                            rendered.push_str(&replacer(expression));
                            cursor += width;
                            index = cursor;
                            break;
                        }
                    }
                    _ => {}
                }
                cursor += width;
            }
            continue;
        }

        let ch = template[index..].chars().next().unwrap();
        rendered.push(ch);
        index += ch.len_utf8();
    }

    decode_js_escapes(&rendered)
}

fn decode_js_escapes(raw: &str) -> String {
    let mut decoded = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') => decoded.push('\n'),
            Some('r') => decoded.push('\r'),
            Some('t') => decoded.push('\t'),
            Some('\n') => {}
            Some('\r') => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            Some('\'') => decoded.push('\''),
            Some('"') => decoded.push('"'),
            Some('`') => decoded.push('`'),
            Some('\\') => decoded.push('\\'),
            Some('u') => {
                let mut code = String::new();
                for _ in 0..4 {
                    code.push(chars.next().expect("unicode escape"));
                }
                let scalar = u32::from_str_radix(&code, 16).expect("valid unicode escape");
                decoded.push(char::from_u32(scalar).expect("unicode scalar"));
            }
            Some(other) => {
                decoded.push('\\');
                decoded.push(other);
            }
            None => decoded.push('\\'),
        }
    }

    decoded
}

fn normalize_inline_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_prompt_lines(raw: &str) -> String {
    let mut normalized = Vec::new();
    let mut previous_blank = false;

    for line in raw.lines().map(str::trim_end) {
        if line.is_empty() {
            if !previous_blank {
                normalized.push(String::new());
            }
            previous_blank = true;
            continue;
        }

        normalized.push(line.to_string());
        previous_blank = false;
    }

    normalized.join("\n")
}

fn indent_block(block: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    block
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn reference_explore_agent_description() -> String {
    "Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. \"src/components/**/*.tsx\"), search code for keywords (eg. \"API endpoints\"), or answer questions about the codebase (eg. \"how do API endpoints work?\"). When calling this agent, specify the desired thoroughness level: \"quick\" for basic searches, \"medium\" for moderate exploration, or \"very thorough\" for comprehensive analysis across multiple locations and naming conventions.".to_string()
}

fn reference_explore_agent_prompt() -> String {
    let reference =
        read_repo_file("references/claude-code/src/tools/AgentTool/built-in/exploreAgent.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
        .replace("${BASH_TOOL_NAME}", "Bash")
        .replace("${FILE_READ_TOOL_NAME}", "Read")
        .replace(
            "${globGuidance}",
            "- Use `Glob` for broad file pattern matching",
        )
        .replace(
            "${grepGuidance}",
            "- Use `Grep` for searching file contents with regex",
        )
        .replace("${embedded ? ', grep' : ''}", "")
}

fn reference_general_purpose_agent_description() -> String {
    "General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you.".to_string()
}

fn reference_general_purpose_agent_prompt() -> String {
    let reference = read_repo_file(
        "references/claude-code/src/tools/AgentTool/built-in/generalPurposeAgent.ts",
    );
    let prefix = normalize_reference_template(&extract_template_literal(
        &reference,
        "const SHARED_PREFIX = `",
    ));
    let guidelines = normalize_reference_template(&extract_template_literal(
        &reference,
        "const SHARED_GUIDELINES = `",
    ));
    format!(
        "{prefix} When you complete the task, respond with a concise report covering what was done and any key findings — the caller will relay this to the user, so it only needs the essentials.\n\n{guidelines}"
    )
}

fn reference_plan_agent_description() -> String {
    "Software architect agent for designing implementation plans. Use this when you need to plan the implementation strategy for a task. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs.".to_string()
}

fn reference_plan_agent_prompt() -> String {
    let reference =
        read_repo_file("references/claude-code/src/tools/AgentTool/built-in/planAgent.ts");
    normalize_reference_template(&extract_template_literal(&reference, "  return `"))
        .replace("${searchToolsHint}", "Glob, Grep, and Read")
        .replace("${BASH_TOOL_NAME}", "Bash")
        .replace("${hasEmbeddedSearchTools() ? ', grep' : ''}", "")
}

fn reference_statusline_setup_agent_description() -> String {
    "Use this agent to configure the user's Claude Code status line setting.".to_string()
}

fn reference_statusline_setup_agent_prompt() -> String {
    let reference =
        read_repo_file("references/claude-code/src/tools/AgentTool/built-in/statuslineSetup.ts");
    trim_line_trailing_whitespace(&decode_js_template_escapes(&normalize_reference_template(
        &extract_template_literal(&reference, "const STATUSLINE_SYSTEM_PROMPT = `"),
    )))
}

fn reference_verification_agent_description() -> String {
    "Use this agent to verify that implementation work is correct before reporting completion. Invoke after non-trivial tasks (3+ file edits, backend/API changes, infrastructure changes). Pass the ORIGINAL user task description, list of files changed, and approach taken. The agent runs builds, tests, linters, and checks to produce a PASS/FAIL/PARTIAL verdict with evidence.".to_string()
}

fn reference_verification_agent_prompt() -> String {
    let reference =
        read_repo_file("references/claude-code/src/tools/AgentTool/built-in/verificationAgent.ts");
    trim_line_trailing_whitespace(&decode_js_template_escapes(
        &normalize_reference_template(&extract_template_literal(
            &reference,
            "const VERIFICATION_SYSTEM_PROMPT = `",
        ))
        .replace("${BASH_TOOL_NAME}", "Bash")
        .replace("${WEB_FETCH_TOOL_NAME}", "WebFetch"),
    ))
}
