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

    let args: Vec<String> = env::args().collect();
    let api_key: String;
    if args.len() > 1 {
        api_key = args[1].clone();
    } else {
        api_key = match env::var("GEMINI_API_KEY") {
            Ok(api_key) => api_key,
            Err(e) => {
                println!("make sure setting api key {}",e);

                return Ok(());
            },
        };

    }

    let prompto = create_prompt(&diff);
    let message = generate_commit_message(&prompto,api_key).await?;

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
Please generate a concise yet appropriate commit message based on the provided Git diff, following Conventional Commits.
The key words “MUST”, “MUST NOT”, “REQUIRED”, “SHALL”, “SHALL NOT”, “SHOULD”, “SHOULD NOT”, “RECOMMENDED”, “MAY”, and “OPTIONAL” in this document are to be interpreted as described in RFC 2119.

1. Commits MUST be prefixed with a type, which consists of a noun, feat, fix, etc., followed by the OPTIONAL scope, OPTIONAL !, and REQUIRED terminal colon and space.
2. The type feat MUST be used when a commit adds a new feature to your application or library.
3. The type fix MUST be used when a commit represents a bug fix for your application.
4. A scope MAY be provided after a type. A scope MUST consist of a noun describing a section of the codebase surrounded by parenthesis, e.g., fix(parser):
5. A description MUST immediately follow the colon and space after the type/scope prefix. The description is a short summary of the code changes, e.g., fix: array parsing issue when multiple spaces were contained in string.
6. A longer commit body MAY be provided after the short description, providing additional contextual information about the code changes. The body MUST begin one blank line after the description.
7. A commit body is free-form and MAY consist of any number of newline separated paragraphs.
8. One or more footers MAY be provided one blank line after the body. Each footer MUST consist of a word token, followed by either a :<space> or <space># separator, followed by a string value (this is inspired by the git trailer convention).
9. A footer’s token MUST use - in place of whitespace characters, e.g., Acked-by (this helps differentiate the footer section from a multi-paragraph body). An exception is made for BREAKING CHANGE, which MAY also be used as a token.
10. A footer’s value MAY contain spaces and newlines, and parsing MUST terminate when the next valid footer token/separator pair is observed.
11. Breaking changes MUST be indicated in the type/scope prefix of a commit, or as an entry in the footer.
12. If included as a footer, a breaking change MUST consist of the uppercase text BREAKING CHANGE, followed by a colon, space, and description, e.g., BREAKING CHANGE: environment variables now take precedence over config files.
13. If included in the type/scope prefix, breaking changes MUST be indicated by a ! immediately before the :. If ! is used, BREAKING CHANGE: MAY be omitted from the footer section, and the commit description SHALL be used to describe the breaking change.
14. Types other than feat and fix MAY be used in your commit messages, e.g., docs: update ref docs.
15. The units of information that make up Conventional Commits MUST NOT be treated as case sensitive by implementors, with the exception of BREAKING CHANGE which MUST be uppercase.
16. BREAKING-CHANGE MUST be synonymous with BREAKING CHANGE, when used as a token in a footer.
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

async fn generate_commit_message(prompt: &str, api_key: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={}",api_key
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
