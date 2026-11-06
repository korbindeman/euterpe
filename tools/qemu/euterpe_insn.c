/*
 * QEMU TCG plugin for `euterpe bench`: counts guest instructions between
 * marker functions in the bench firmware.
 *
 * Arguments (addresses of the firmware's marker functions, from its symbol
 * table; the Thumb bit is ignored):
 *   start=0x...  begin counting
 *   stop=0x...   stop counting, add to the running total
 *   end=0x...    print "euterpe-bench <total> <regions>" and reset the total
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 * (QEMU plugins are loaded into QEMU, which is GPL-2.0.)
 */
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <qemu-plugin.h>

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

static uint64_t insns;      /* every guest instruction, counted inline */
static uint64_t region_at;  /* insns at the last start marker */
static uint64_t total;      /* sum over regions since the last end marker */
static uint64_t regions;
static uint64_t addr_start, addr_stop, addr_end;

static void on_start(unsigned int vcpu, void *udata)
{
    region_at = insns;
}

static void on_stop(unsigned int vcpu, void *udata)
{
    total += insns - region_at;
    regions++;
}

static void on_end(unsigned int vcpu, void *udata)
{
    char line[96];
    snprintf(line, sizeof line, "euterpe-bench %" PRIu64 " %" PRIu64 "\n",
             total, regions);
    qemu_plugin_outs(line);
    total = 0;
    regions = 0;
}

static void on_tb_trans(qemu_plugin_id_t id, struct qemu_plugin_tb *tb)
{
    size_t n = qemu_plugin_tb_n_insns(tb);
    for (size_t i = 0; i < n; i++) {
        struct qemu_plugin_insn *insn = qemu_plugin_tb_get_insn(tb, i);
        uint64_t pc = qemu_plugin_insn_vaddr(insn);
        if (pc == addr_start) {
            qemu_plugin_register_vcpu_insn_exec_cb(insn, on_start,
                                                   QEMU_PLUGIN_CB_NO_REGS, NULL);
        } else if (pc == addr_stop) {
            qemu_plugin_register_vcpu_insn_exec_cb(insn, on_stop,
                                                   QEMU_PLUGIN_CB_NO_REGS, NULL);
        } else if (pc == addr_end) {
            qemu_plugin_register_vcpu_insn_exec_cb(insn, on_end,
                                                   QEMU_PLUGIN_CB_NO_REGS, NULL);
        }
        qemu_plugin_register_vcpu_insn_exec_inline(insn, QEMU_PLUGIN_INLINE_ADD_U64,
                                                   &insns, 1);
    }
}

static int parse_addr(const char *arg, const char *key, uint64_t *out)
{
    size_t k = strlen(key);
    if (strncmp(arg, key, k) != 0 || arg[k] != '=') {
        return 0;
    }
    *out = strtoull(arg + k + 1, NULL, 0) & ~(uint64_t)1;
    return 1;
}

QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id,
                                           const qemu_info_t *info,
                                           int argc, char **argv)
{
    for (int i = 0; i < argc; i++) {
        if (!parse_addr(argv[i], "start", &addr_start) &&
            !parse_addr(argv[i], "stop", &addr_stop) &&
            !parse_addr(argv[i], "end", &addr_end)) {
            fprintf(stderr, "euterpe_insn: unknown argument %s\n", argv[i]);
            return -1;
        }
    }
    if (!addr_start || !addr_stop || !addr_end) {
        fprintf(stderr, "euterpe_insn: start=, stop= and end= are required\n");
        return -1;
    }
    qemu_plugin_register_vcpu_tb_trans_cb(id, on_tb_trans);
    return 0;
}
