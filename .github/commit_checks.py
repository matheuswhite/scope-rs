"""
 :file: commit_checks.py
 :author: Paulo Santos (pauloroberto.santos@edge.ufal.br)
 :brief: Realiza a verificação do histórico de commits.
 :version: 0.1
 :date: 29-04-2024

 :copyright: Copyright (c) 2024
"""

import argparse
import subprocess
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-s", "--srcb", metavar="sbranch", help="Source branch to compare.")
    parser.add_argument("-t", "--tgtb", metavar="tbranch", help="Target branch to compare.",
                        default="origin/main")

    source_branch = str(parser.parse_args().srcb)
    target_branch = str(parser.parse_args().tgtb)

    if source_branch == "None":
        print("\nSource branch is empty.")
        return -1

    invocation = f"git rev-list --count --left-right {target_branch}...{source_branch}"

    try:
        out = subprocess.check_output(invocation, shell=True)
        behind, ahead = [int(dist) for dist in out.decode("utf-8").rsplit("\t", maxsplit=1)]
    except Exception as e:
        print(f"\nError to check the history: {e}")
        return -1

    if behind > 0:
        print(f"\nThere are {behind} commits to be included in PR using rebase to main.\n")

        return 1

    if ahead > COMMIT_LIMIT:
        print(f"\nThere are {ahead} commits to be added, divide PR to have less than {COMMIT_LIMIT + 1} commits.")

        return 1

    print(f"\nHistory is updated. ({ahead} commits ahead)")

    invocation = f"git describe --all --first-parent {source_branch}~1 --abbrev=0"

    try:
        out = subprocess.check_output(invocation, shell=True)
        parent_head = out.decode("utf-8").split("/")[-1].strip()
    except Exception as e:
        print(f"\nError to check the history: {e}")

        return -1

    if parent_head != "main":
        print(
            f"\nThe branch has its origin on `{parent_head}`, resolve `{parent_head}` first. (the branch must have "
            f"origin on main)")

        return 1

    print("\nThe branch has its origin on main.")

    return 0


if __name__ == "__main__":
    COMMIT_LIMIT = 15
    check_status = main()

    sys.exit(check_status)
