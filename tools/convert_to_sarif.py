"""Convert the output of lintrunner json to SARIF."""

import argparse
import json
import os


def format_rule_name(lintrunner_result: dict) -> str:
    return f"{lintrunner_result['code']}/{lintrunner_result['name']}"


def severity_to_github_level(severity: str) -> str:
    if severity == "advice" or severity == "disabled":
        return "warning"
    return severity


def parse_single_lintrunner_result(lintrunner_result: dict) -> tuple:
    r"""Parse a single lintrunner result.

    A result looks like this:
    {
        "path":"/adapters/pytorch/grep_linter.py",
        "line":227,
        "char":80,
        "code":"FLAKE8",
        "severity":"advice",
        "name":"E501",
        "description":"line too long (81 > 79 characters)\nSee https://www.flake8rules.com/rules/E501.html"
    }
    """
    result = {
        "ruleId": format_rule_name(lintrunner_result),
        "level": severity_to_github_level(lintrunner_result["severity"]),
        "message": {
            "text": format_rule_name(lintrunner_result)
            + "\n"
            + lintrunner_result["description"],
        },
        "locations": [
            {
                "physicalLocation": {
                    "artifactLocation": {
                        "uri": "file://" + lintrunner_result["path"],
                    },
                    "region": {
                        "startLine": lintrunner_result["line"] or 1,
                        "startColumn": lintrunner_result["char"] or 1,
                    },
                },
            },
        ],
    }

    rule = {
        "id": format_rule_name(lintrunner_result),
        "rule": {
            "id": format_rule_name(lintrunner_result),
            "name": format_rule_name(lintrunner_result),
            "shortDescription": {
                "text": format_rule_name(lintrunner_result)
                + ": "
                + lintrunner_result["description"].split("\n")[0],
            },
            "fullDescription": {
                "text": format_rule_name(lintrunner_result)
                + "\n"
                + lintrunner_result["description"],
            },
            "defaultConfiguration": {
                "level": severity_to_github_level(lintrunner_result["severity"]),
            },
        },
    }

    return result, rule


def main(args):
    """Convert the output of lintrunner json to SARIF."""

    rules = {}
    results = []
    with open(args.input, "r") as f:
        for line in f:
            lintrunner_json = json.loads(line)
            result, rule = parse_single_lintrunner_result(lintrunner_json)
            results.append(result)
            rules[rule["id"]] = rule["rule"]

    sarif = {
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [
            {
                "tool": {
                    "driver": {
                        "name": "lintrunner",
                        "rules": list(rules.values()),
                    },
                },
                "results": results,
            },
        ],
    }

    output_dir = os.path.dirname(args.output)
    if output_dir:
        os.makedirs(os.path.dirname(args.output), exist_ok=True)

    with open(args.output, "w") as f:
        json.dump(sarif, f)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=str, required=True)
    parser.add_argument("--output", type=str, required=True)
    args = parser.parse_args()
    main(args)
