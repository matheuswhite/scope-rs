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
    parser.add_argument("-s", "--srcb", metavar="sbranch", help="Branch de origem da comparação.")
    parser.add_argument("-t", "--tgtb", metavar="tbranch", help="Branch de destino da comparação.",
                        default="origin/main")

    source_branch = str(parser.parse_args().srcb)
    target_branch = str(parser.parse_args().tgtb)

    if source_branch == "None":
        print("\nBranch a ser verificada não fornecida.")
        return -1

    invocation = f"git rev-list --count --left-right {target_branch}...{source_branch}"

    try:
        out = subprocess.check_output(invocation, shell=True)
        behind, ahead = [int(dist) for dist in out.decode("utf-8").rsplit("\t", maxsplit=1)]
    except Exception as e:
        print(f"\nErro ao verificar o histórico: {e}")
        return -1

    if behind > 0:
        print(f"\nHá {behind} commits que precisam ser incorporados no PR por rebase à main.\n")

        return 1

    if ahead > COMMIT_LIMIT:
        print(f"\nHá {ahead} commits a serem adicionados, divida o PR para que contenha menos que {COMMIT_LIMIT + 1}"
              " commits.")

        return 1

    print(f"\nHistórico está atualizado. ({ahead} commits a frente)")

    invocation = f"git describe --all --first-parent {source_branch}~1 --abbrev=0"

    try:
        out = subprocess.check_output(invocation, shell=True)
        parent_head = out.decode("utf-8").split("/")[-1].strip()
    except Exception as e:
        print(f"\nErro ao verificar o histórico: {e}")

        return -1

    if parent_head != "main":
        print(f"\nA branch tem origem em `{parent_head}`, resolva a `{parent_head}` primeiro. (A branch deve ter origem "
              "na main)")

        return 1

    print("\nA branch tem origem na main.")

    return 0


if __name__ == "__main__":
    COMMIT_LIMIT = 15
    check_status = main()

    sys.exit(check_status)
