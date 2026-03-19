#!/usr/bin/env python3
"""
playwright_cdp.py -- Stateless browser automation bridge for CloseClaw.

Usage:
    python3 playwright_cdp.py <cdp_url> '<action_json>'

Connects to a running Chrome/Edge via CDP, performs one action, returns JSON.
Chrome lifecycle is managed externally by the Rust BrowserTool.
"""

import sys
import json
import traceback
from playwright.sync_api import sync_playwright

MAX_TEXT_LENGTH = 30000


def main():
    if len(sys.argv) < 3:
        print(json.dumps({"error": "Usage: playwright_cdp.py <cdp_url> '<action_json>'"}))
        sys.exit(1)

    cdp_url = sys.argv[1]
    try:
        action = json.loads(sys.argv[2])
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"Invalid JSON: {e}"}))
        sys.exit(1)

    with sync_playwright() as p:
        try:
            browser = p.chromium.connect_over_cdp(cdp_url)
        except Exception as e:
            print(json.dumps({"error": f"Failed to connect to CDP at {cdp_url}: {e}"}))
            sys.exit(1)

        try:
            result = dispatch(browser, action)
            output = json.dumps(result, ensure_ascii=False, default=str)
            # Truncate if output is extremely large
            if len(output) > MAX_TEXT_LENGTH:
                output = output[:MAX_TEXT_LENGTH] + '..."}'
            print(output)
        except Exception as e:
            print(json.dumps({
                "error": str(e),
                "traceback": traceback.format_exc()
            }))
        finally:
            browser.close()  # disconnects from CDP, does NOT close Chrome


def get_page(browser, tab_index):
    """Get a page by tab index from the first browser context."""
    contexts = browser.contexts
    if not contexts:
        return None, {"error": "No browser contexts found"}
    pages = contexts[0].pages
    if not pages:
        return None, {"error": "No pages/tabs open"}
    if tab_index >= len(pages):
        return None, {"error": f"tab_index {tab_index} out of range (have {len(pages)} tabs)"}
    return pages[tab_index], None


def dispatch(browser, action):
    """Route action to handler. Returns dict."""
    act = action.get("action")
    params = action.get("params", {})

    # Extract tab_index before passing params to handlers
    tab_index = params.pop("tab_index", 0)

    # Actions that operate on browser context (not a specific page)
    if act == "list_tabs":
        return handle_list_tabs(browser, params)
    if act == "new_tab":
        return handle_new_tab(browser, params)

    # All other actions need a page
    handlers = {
        "navigate": handle_navigate,
        "click": handle_click,
        "type": handle_type,
        "press_key": handle_press_key,
        "screenshot": handle_screenshot,
        "get_text": handle_get_text,
        "get_html": handle_get_html,
        "get_attribute": handle_get_attribute,
        "query_selector_all": handle_query_selector_all,
        "wait_for_selector": handle_wait_for_selector,
        "wait_for_navigation": handle_wait_for_navigation,
        "evaluate": handle_evaluate,
        "select_option": handle_select_option,
        "check": handle_check,
        "upload_file": handle_upload_file,
        "scroll": handle_scroll,
        "close_tab": handle_close_tab,
    }

    handler = handlers.get(act)
    if not handler:
        return {"error": f"Unknown action: {act}"}

    page, err = get_page(browser, tab_index)
    if err:
        return err

    return handler(page, params)


# --- Handlers ---

def handle_list_tabs(browser, params):
    tabs = []
    for ctx in browser.contexts:
        for i, page in enumerate(ctx.pages):
            tabs.append({"index": i, "url": page.url, "title": page.title()})
    return {"tabs": tabs}


def handle_navigate(page, params):
    url = params.get("url")
    if not url:
        return {"error": "Missing required param: url"}
    timeout = params.get("timeout", 30000)
    page.goto(url, timeout=timeout, wait_until="domcontentloaded")
    return {"url": page.url, "title": page.title()}


def handle_click(page, params):
    selector = params.get("selector")
    if not selector:
        return {"error": "Missing required param: selector"}
    timeout = params.get("timeout", 5000)
    page.click(selector, timeout=timeout)
    return {"clicked": selector}


def handle_type(page, params):
    selector = params.get("selector")
    text = params.get("text")
    if not selector or text is None:
        return {"error": "Missing required params: selector, text"}
    clear = params.get("clear", False)
    delay = params.get("delay", 50)
    if clear:
        page.fill(selector, "")
    page.type(selector, text, delay=delay)
    return {"typed": len(text), "selector": selector}


def handle_press_key(page, params):
    key = params.get("key")
    if not key:
        return {"error": "Missing required param: key"}
    page.keyboard.press(key)
    return {"pressed": key}


def handle_screenshot(page, params):
    path = params.get("path")
    if not path:
        return {"error": "Missing required param: path"}
    full_page = params.get("full_page", False)
    selector = params.get("selector")
    if selector:
        element = page.query_selector(selector)
        if element:
            element.screenshot(path=path)
        else:
            return {"error": f"Selector not found for screenshot: {selector}"}
    else:
        page.screenshot(path=path, full_page=full_page)
    return {"screenshot": path}


def handle_get_text(page, params):
    selector = params.get("selector", "body")
    timeout = params.get("timeout", 5000)
    max_len = params.get("max_length", MAX_TEXT_LENGTH)
    element = page.wait_for_selector(selector, timeout=timeout)
    text = element.inner_text() if element else ""
    if len(text) > max_len:
        text = text[:max_len] + f"\n... [truncated, {len(text)} chars total]"
    return {"text": text}


def handle_get_html(page, params):
    selector = params.get("selector", "body")
    outer = params.get("outer", False)
    max_len = params.get("max_length", MAX_TEXT_LENGTH)
    element = page.query_selector(selector)
    if not element:
        return {"error": f"Selector not found: {selector}"}
    if outer:
        html = element.evaluate("e => e.outerHTML")
    else:
        html = element.evaluate("e => e.innerHTML")
    if len(html) > max_len:
        html = html[:max_len] + "\n... [truncated]"
    return {"html": html}


def handle_get_attribute(page, params):
    selector = params.get("selector")
    attribute = params.get("attribute")
    if not selector or not attribute:
        return {"error": "Missing required params: selector, attribute"}
    element = page.query_selector(selector)
    if not element:
        return {"error": f"Selector not found: {selector}"}
    value = element.get_attribute(attribute)
    return {"attribute": attribute, "value": value}


def handle_query_selector_all(page, params):
    selector = params.get("selector")
    if not selector:
        return {"error": "Missing required param: selector"}
    max_results = params.get("max_results", 50)
    elements = page.query_selector_all(selector)
    results = []
    for el in elements[:max_results]:
        text = el.inner_text()
        if len(text) > 200:
            text = text[:200] + "..."
        results.append({
            "tag": el.evaluate("e => e.tagName.toLowerCase()"),
            "text": text,
            "visible": el.is_visible(),
        })
    return {"count": len(elements), "results": results}


def handle_wait_for_selector(page, params):
    selector = params.get("selector")
    if not selector:
        return {"error": "Missing required param: selector"}
    timeout = params.get("timeout", 10000)
    state = params.get("state", "visible")
    page.wait_for_selector(selector, timeout=timeout, state=state)
    return {"found": selector, "state": state}


def handle_wait_for_navigation(page, params):
    timeout = params.get("timeout", 30000)
    page.wait_for_load_state("domcontentloaded", timeout=timeout)
    return {"url": page.url, "title": page.title()}


def handle_evaluate(page, params):
    expression = params.get("expression")
    if not expression:
        return {"error": "Missing required param: expression"}
    result = page.evaluate(expression)
    return {"result": result}


def handle_select_option(page, params):
    selector = params.get("selector")
    if not selector:
        return {"error": "Missing required param: selector"}
    value = params.get("value")
    label = params.get("label")
    if label:
        page.select_option(selector, label=label)
        return {"selected_label": label}
    elif value:
        page.select_option(selector, value=value)
        return {"selected_value": value}
    return {"error": "Provide 'value' or 'label'"}


def handle_check(page, params):
    selector = params.get("selector")
    if not selector:
        return {"error": "Missing required param: selector"}
    checked = params.get("checked", True)
    if checked:
        page.check(selector)
    else:
        page.uncheck(selector)
    return {"checked": checked, "selector": selector}


def handle_upload_file(page, params):
    selector = params.get("selector")
    file_path = params.get("file_path")
    if not selector or not file_path:
        return {"error": "Missing required params: selector, file_path"}
    page.set_input_files(selector, file_path)
    return {"uploaded": file_path}


def handle_scroll(page, params):
    direction = params.get("direction", "down")
    amount = params.get("amount", 500)
    if direction == "down":
        page.evaluate(f"window.scrollBy(0, {amount})")
    elif direction == "up":
        page.evaluate(f"window.scrollBy(0, -{amount})")
    elif direction == "left":
        page.evaluate(f"window.scrollBy(-{amount}, 0)")
    elif direction == "right":
        page.evaluate(f"window.scrollBy({amount}, 0)")
    return {"scrolled": direction, "amount": amount}


def handle_new_tab(browser, params):
    url = params.get("url", "about:blank")
    contexts = browser.contexts
    if not contexts:
        return {"error": "No browser contexts"}
    page = contexts[0].new_page()
    if url != "about:blank":
        page.goto(url, wait_until="domcontentloaded")
    return {"new_tab_index": len(contexts[0].pages) - 1, "url": page.url}


def handle_close_tab(page, params):
    url = page.url
    page.close()
    return {"closed": url}


if __name__ == "__main__":
    main()
