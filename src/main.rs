use arboard::Clipboard;
use core::str;
use dotenvy::dotenv;
use std::env;
use git2::Repository;
use reqwest::Client;
use serde::Deserialize;

fn get_git_diff() -> Result<String, Box<dyn std::error::Error>> {
    let repo = match Repository::open(env::current_dir().unwrap()) {
        Ok(repo) => repo,
        Err(e) => panic!("faild to open: {}", e),
    };

    let head = repo.head()?.peel_to_tree()?;
    let diff = repo.diff_tree_to_index(Some(&head), Some(&repo.index()?), None)?;

    let mut diff_text_vec = Vec::new();
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        diff_text_vec.extend_from_slice(line.content());
        true
    })?;

    let diff_text_string = String::from_utf8(diff_text_vec)?;
    Ok(diff_text_string)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().expect(".env file not fount");
    let diff : String= match get_git_diff() {
        Ok(message) => message,
        Err(e) => {
            println!("error get_git_diff {}",e);
            return Ok(());
        },
    };
    if diff == "" {
        println!("Nothing to commit");
        return Ok(());
    }

    let prompto = create_prompt(&diff);
    let message = generate_commit_message(&prompto).await?;

    println!("{:?}",message);

    match copy_to_clip(&message) {
        Ok(_) => println!("success to copy to clip"),
        Err(e) => eprintln!("fail to copy to clip {:?}",e), 
    }
    Ok(())
}

fn copy_to_clip(message: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(message)?;
    Ok(())
}


const COMMIT_MESSAGE_GUIDELINE: &str = r#"
Please generate a concise yet appropriate commit message based on the provided Git diff, following the commit message guidelines below.
use Semantic Commit Messages.
## Guidelines
1. **Format:** Use the format `<type>(<scope>): <subject>`.
2. **type:** Indicate the type of change. The main types are:
- `feat`: Addition of new features
- `fix`: Bug fixes
- `docs`: Documentation-only changes
- `style`: Code style changes (spacing, formatting, etc.)
- `refactor`: Refactoring (does not include bug fixes or new features)
- `test`: Addition or modification of tests
- `chore`: Changes to the build process or supporting tools
3. **subject:** 50 characters or less, written in the present tense (e.g., "to fix" instead of "fixed").
4. **Body (Optional):** If you need a more detailed explanation, add it on a new line.

## Generation Request
Analyze the diff content and return only the **compliant commit message body**. Do not include any other explanations or comments.
"#;

fn create_prompt(diff: &str) -> String {
    format!(
        "{}\n\n---\n\n## Git Diff\n\n```diff\n{}\n```",
        COMMIT_MESSAGE_GUIDELINE,
        diff
    )
}

#[derive(Deserialize, Debug)]
struct Part {
    text: String,
}

#[derive(Deserialize, Debug)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Deserialize, Debug)]
struct Candidate {
    content: Option<Content>, 
    finish_reason: Option<String>, 
    safety_ratings: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
    prompt_feedback: Option<serde_json::Value>,
}

async fn generate_commit_message(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let api_key: String = match env::var("GEMINI_API_KEY") {
        Ok(api_key) => api_key,
        Err(e) => {
            println!("make sure setting api key {}",e);
            return Ok("no api key".to_string());
        },
    };

    let client = Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={}",
        api_key
    );

    let payload = serde_json::json!({
        "contents": [
        {
            "parts": [
            {"text": prompt}
            ]
        }
        ],
    });

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?; 
    let body: GeminiResponse = response.json().await?;

    let commit_message = body.candidates.get(0)
        .and_then(|c| c.content.as_ref())
        .and_then(|content| content.parts.get(0))
        .map(|part| part.text.trim().to_string());


    match commit_message {
        Some(text) => Ok(text),
        None => {
            let reason = body.candidates.get(0)
                .and_then(|c| c.finish_reason.as_ref())
                .unwrap_or(&"不明 (candidatesが空か構造不正)".to_string())
                .clone();
            
            let feedback_info = body.prompt_feedback
                .map(|f| format!("Prompt Feedback: {:?}", f))
                .unwrap_or_else(|| "No Prompt Feedback".to_string());

            Err(format!(
                "Gemini APIは有効なテキストを返しませんでした。\n\
                 原因: finish_reason='{}'\n\
                 詳細: {}",
                reason,
                feedback_info
            ).into())
        }
    }
}
