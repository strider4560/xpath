/* Flat planning projection of the compiled libyang v2.1.148 schema tree.
 * Only this import-time program links libyang; the JSON consumer does not.
 */
#define _POSIX_C_SOURCE 200809L
#include <libyang/libyang.h>
#include <libyang/version.h>

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* LY_VERSION is the library ABI version, not the release tag. */
#if LY_VERSION_MAJOR != 2 || LY_VERSION_MINOR != 46 || LY_VERSION_MICRO != 3
#error "Build against libyang v2.1.148 (ABI 2.46.3). See README.md."
#endif
#define COMPILER_NAME "libyang 2.1.148"
#define MAX_PROJECTION_ROWS UINT32_C(1000000) /* Includes the virtual root. */
#define MAX_SCHEMA_DEPTH 255 /* Virtual root is depth 0; 256 rows per path. */

struct row {
    const struct lysc_node *node;
    uint32_t parent;
};

struct projection {
    struct row *rows;
    size_t row_count;
    size_t row_capacity;
    const struct lys_module **modules;
    size_t module_count;
};

struct feature_request {
    char *module;
    const char *name;
};

static void usage(FILE *stream)
{
    fputs("Usage: schema-export [--search-dir DIR] [--feature MODULE:FEATURE] MODULE.yang...\n"
          "Repeat either option as needed. Features are disabled unless requested.\n"
          "Writes FORMAT.md version 1 JSON to stdout; diagnostics go to stderr.\n",
          stream);
}

static void json_string(const char *value)
{
    if (!value) {
        fputs("null", stdout);
        return;
    }
    putchar('"');
    for (const unsigned char *p = (const unsigned char *)value; *p; ++p) {
        switch (*p) {
        case '"': fputs("\\\"", stdout); break;
        case '\\': fputs("\\\\", stdout); break;
        case '\b': fputs("\\b", stdout); break;
        case '\f': fputs("\\f", stdout); break;
        case '\n': fputs("\\n", stdout); break;
        case '\r': fputs("\\r", stdout); break;
        case '\t': fputs("\\t", stdout); break;
        default:
            if (*p < 0x20) {
                printf("\\u%04x", (unsigned)*p);
            } else {
                putchar(*p); /* libyang validates YANG input as UTF-8. */
            }
        }
    }
    putchar('"');
}

static const char *boolean(int value)
{
    return value ? "true" : "false";
}

static const char *kind_name(uint16_t kind)
{
    switch (kind) {
    case LYS_CONTAINER: return "container";
    case LYS_LIST: return "list";
    case LYS_LEAF: return "leaf";
    case LYS_LEAFLIST: return "leaf-list";
    case LYS_CHOICE: return "choice";
    case LYS_CASE: return "case";
    case LYS_ANYDATA: return "anydata";
    case LYS_ANYXML: return "anyxml";
    default: return NULL;
    }
}

static const char *base_type(const struct lysc_node *node)
{
    const struct lysc_type *type = NULL;
    if (node->nodetype == LYS_LEAF) {
        type = ((const struct lysc_node_leaf *)node)->type;
    } else if (node->nodetype == LYS_LEAFLIST) {
        type = ((const struct lysc_node_leaflist *)node)->type;
    }
    if (!type) {
        return NULL;
    }
    switch (type->basetype) {
    case LY_TYPE_BINARY: return "binary";
    case LY_TYPE_UINT8: return "uint8";
    case LY_TYPE_UINT16: return "uint16";
    case LY_TYPE_UINT32: return "uint32";
    case LY_TYPE_UINT64: return "uint64";
    case LY_TYPE_STRING: return "string";
    case LY_TYPE_BITS: return "bits";
    case LY_TYPE_BOOL: return "boolean";
    case LY_TYPE_DEC64: return "decimal64";
    case LY_TYPE_EMPTY: return "empty";
    case LY_TYPE_ENUM: return "enumeration";
    case LY_TYPE_IDENT: return "identityref";
    case LY_TYPE_INST: return "instance-identifier";
    case LY_TYPE_LEAFREF: return "leafref";
    case LY_TYPE_UNION: return "union";
    case LY_TYPE_INT8: return "int8";
    case LY_TYPE_INT16: return "int16";
    case LY_TYPE_INT32: return "int32";
    case LY_TYPE_INT64: return "int64";
    default: return NULL;
    }
}

static int compare_strings(const void *a, const void *b)
{
    return strcmp(*(const char *const *)a, *(const char *const *)b);
}

static int compare_modules(const void *a, const void *b)
{
    return strcmp((*(const struct lys_module *const *)a)->name,
                  (*(const struct lys_module *const *)b)->name);
}

static int add_module(struct projection *p, const struct lys_module *module)
{
    for (size_t i = 0; i < p->module_count; ++i) {
        if (p->modules[i] == module) {
            return 0;
        }
        if (!strcmp(p->modules[i]->name, module->name)) {
            fprintf(stderr, "Multiple revisions of module %s cannot be represented.\n", module->name);
            return -1;
        }
    }
    const struct lys_module **modules = realloc(p->modules, (p->module_count + 1) * sizeof *modules);
    if (!modules) {
        return -1;
    }
    p->modules = modules;
    p->modules[p->module_count++] = module;
    return 0;
}

static int collect_node(struct projection *p, const struct lysc_node *node, uint32_t parent, unsigned depth)
{
    if (depth > MAX_SCHEMA_DEPTH) {
        fprintf(stderr, "Schema depth exceeds 256 path rows including the virtual root at %s.\n", node->name);
        return -1;
    }
    if (p->row_count >= MAX_PROJECTION_ROWS) {
        fprintf(stderr, "Schema exceeds 1000000 total rows including the virtual root at %s.\n", node->name);
        return -1;
    }
    if (!kind_name(node->nodetype) ||
            (((node->nodetype == LYS_LEAF) || (node->nodetype == LYS_LEAFLIST)) && !base_type(node))) {
        fprintf(stderr, "Unsupported node or base type at %s.\n", node->name);
        return -1;
    }
    if (p->row_count == p->row_capacity) {
        size_t capacity = p->row_capacity ? p->row_capacity * 2 : 64;
        if (capacity > SIZE_MAX / sizeof *p->rows) {
            return -1;
        }
        struct row *rows = realloc(p->rows, capacity * sizeof *rows);
        if (!rows) {
            return -1;
        }
        p->rows = rows;
        p->row_capacity = capacity;
    }
    uint32_t id = (uint32_t)p->row_count;
    p->rows[p->row_count++] = (struct row){node, parent};
    if (add_module(p, node->module)) {
        return -1;
    }
    /* Case children share a next chain with other cases. Stop at the physical
     * parent boundary; a plain next-loop would duplicate/reparent those nodes. */
    for (const struct lysc_node *child = lysc_node_child(node);
            child && child->parent == node; child = child->next) {
        if (collect_node(p, child, id, depth + 1)) {
            return -1;
        }
    }
    return 0;
}

static int emit_module(const struct lys_module *module)
{
    const char **features = NULL;
    size_t count = 0;
    uint32_t index = 0;
    const struct lysp_feature *feature = NULL;
    while ((feature = lysp_feature_next(feature, module->parsed, &index))) {
        if (lys_feature_value(module, feature->name) == LY_SUCCESS) {
            const char **next = realloc(features, (count + 1) * sizeof *next);
            if (!next) {
                free(features);
                return -1;
            }
            features = next;
            features[count++] = feature->name;
        }
    }
    if (count > 1) {
        qsort(features, count, sizeof *features, compare_strings);
    }
    fputs("    {\"name\":", stdout); json_string(module->name);
    fputs(",\"namespace\":", stdout); json_string(module->ns);
    fputs(",\"prefix\":", stdout); json_string(module->prefix);
    fputs(",\"revision\":", stdout); json_string(module->revision);
    fputs(",\"enabled_features\":[", stdout);
    for (size_t i = 0; i < count; ++i) {
        if (i) { putchar(','); }
        json_string(features[i]);
    }
    fputs("]}", stdout);
    free(features);
    return 0;
}

static void emit_node(const struct row *row, uint32_t id)
{
    const struct lysc_node *n = row->node;
    printf("    {\"id\":%" PRIu32 ",\"parent\":%" PRIu32 ",\"kind\":", id, row->parent);
    json_string(kind_name(n->nodetype));
    fputs(",\"module\":", stdout); json_string(n->module->name);
    fputs(",\"namespace\":", stdout); json_string(n->module->ns);
    fputs(",\"name\":", stdout); json_string(n->name);
    unsigned config = n->flags & LYS_CONFIG_MASK;
    /* In pinned v2.1.148, LYS_PRESENCE is assigned only when the effective
     * parsed container has a presence statement (including uses/refine).
     * Do not infer this from the broader historical header comment. */
    printf(",\"config\":%s,\"key\":%s,\"mandatory\":%s,\"presence\":%s,\"status\":",
           config == LYS_CONFIG_W ? "true" : config == LYS_CONFIG_R ? "false" : "null",
           boolean(n->nodetype == LYS_LEAF && (n->flags & LYS_KEY)),
           /* Key values are required even when the MAND bit is unset. */
           boolean((n->flags & LYS_MAND_TRUE) || (n->nodetype == LYS_LEAF && (n->flags & LYS_KEY))),
           boolean(n->nodetype == LYS_CONTAINER && (n->flags & LYS_PRESENCE)));
    json_string(n->flags & LYS_STATUS_OBSLT ? "obsolete" :
                n->flags & LYS_STATUS_DEPRC ? "deprecated" : "current");
    fputs(",\"type_name\":", stdout); json_string(base_type(n));
    fputs(",\"when\":[", stdout);
    struct lysc_when **whens = lysc_node_when(n);
    for (LY_ARRAY_COUNT_TYPE i = 0; i < LY_ARRAY_COUNT(whens); ++i) {
        if (i) { putchar(','); }
        json_string(lyxp_get_expr(whens[i]->cond));
    }
    fputs("],\"must\":[", stdout);
    struct lysc_must *musts = lysc_node_musts(n);
    for (LY_ARRAY_COUNT_TYPE i = 0; i < LY_ARRAY_COUNT(musts); ++i) {
        if (i) { putchar(','); }
        json_string(lyxp_get_expr(musts[i].cond));
    }
    fputs("]}", stdout);
}

static int add_search_directory(struct ly_ctx *ctx, const char *directory)
{
    LY_ERR err = ly_ctx_set_searchdir(ctx, directory);
    return (err == LY_SUCCESS || err == LY_EEXIST) ? 0 : -1;
}

int main(int argc, char **argv)
{
    int result = EXIT_FAILURE;
    struct ly_ctx *ctx = NULL;
    struct projection p = {0};
    char **paths = calloc((size_t)argc, sizeof *paths);
    const char **search_dirs = calloc((size_t)argc, sizeof *search_dirs);
    struct feature_request *requests = calloc((size_t)argc, sizeof *requests);
    size_t path_count = 0, directory_count = 0, request_count = 0;
    if (!paths || !search_dirs || !requests) {
        goto cleanup;
    }

    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "--help")) {
            usage(stdout);
            result = EXIT_SUCCESS;
            goto cleanup;
        } else if (!strcmp(argv[i], "--search-dir") && i + 1 < argc) {
            search_dirs[directory_count++] = argv[++i];
        } else if (!strcmp(argv[i], "--feature") && i + 1 < argc) {
            char *module = strdup(argv[++i]);
            if (!module) { goto cleanup; }
            char *colon = strchr(module, ':');
            if (!colon || colon == module || !colon[1] || strchr(colon + 1, ':')) {
                fprintf(stderr, "Expected MODULE:FEATURE, got %s.\n", module);
                free(module);
                goto cleanup;
            }
            *colon = '\0';
            requests[request_count++] = (struct feature_request){module, colon + 1};
        } else if (argv[i][0] == '-') {
            fprintf(stderr, "Unknown or incomplete option: %s\n", argv[i]);
            goto cleanup;
        } else if (!argv[i][0]) {
            fputs("A module path must not be empty.\n", stderr);
            goto cleanup;
        } else {
            paths[path_count++] = argv[i];
        }
    }
    if (!path_count) {
        usage(stderr);
        goto cleanup;
    }
    /* Sorting inputs also stabilizes the order in which multiple augmenting
     * modules are compiled, independent of their CLI argument order. */
    qsort(paths, path_count, sizeof *paths, compare_strings);
    if (ly_ctx_new(NULL, LY_CTX_NO_YANGLIBRARY | LY_CTX_EXPLICIT_COMPILE |
                   LY_CTX_DISABLE_SEARCHDIR_CWD, &ctx) != LY_SUCCESS) {
        goto cleanup;
    }
    for (size_t i = 0; i < directory_count; ++i) {
        if (add_search_directory(ctx, search_dirs[i])) { goto cleanup; }
    }
    for (size_t i = 0; i < path_count; ++i) {
        char *directory = strdup(paths[i]);
        if (!directory) { goto cleanup; }
        char *slash = strrchr(directory, '/');
        if (slash) {
            if (slash == directory) { slash[1] = '\0'; } else { *slash = '\0'; }
        } else {
            strcpy(directory, ".");
        }
        int err = add_search_directory(ctx, directory);
        free(directory);
        if (err) { goto cleanup; }
    }
    for (size_t i = 0; i < path_count; ++i) {
        if (lys_parse_path(ctx, paths[i], LYS_IN_YANG, NULL) != LY_SUCCESS) {
            goto cleanup;
        }
    }
    /* Validate every request, then apply each module's full feature list once. */
    for (size_t i = 0; i < request_count; ++i) {
        struct lys_module *module = ly_ctx_get_module_implemented(ctx, requests[i].module);
        if (!module) { module = ly_ctx_get_module_latest(ctx, requests[i].module); }
        LY_ERR err = module ? lys_feature_value(module, requests[i].name) : LY_ENOTFOUND;
        if (!module || (err != LY_SUCCESS && err != LY_ENOT)) {
            fprintf(stderr, "Unknown feature %s:%s.\n", requests[i].module, requests[i].name);
            goto cleanup;
        }
        int already_applied = 0;
        for (size_t j = 0; j < i; ++j) {
            if (!strcmp(requests[j].module, requests[i].module)) { already_applied = 1; }
        }
        if (already_applied) { continue; }
        const char **features = calloc(request_count + 1, sizeof *features);
        if (!features) { goto cleanup; }
        size_t count = 0;
        for (size_t j = 0; j < request_count; ++j) {
            if (!strcmp(requests[j].module, requests[i].module)) {
                features[count++] = requests[j].name;
            }
        }
        err = lys_set_implemented(module, features);
        free(features);
        if (err != LY_SUCCESS) { goto cleanup; }
    }
    if (ly_ctx_compile(ctx) != LY_SUCCESS) { goto cleanup; }

    /* Internal support modules do not describe this requested schema set.
     * If any emitted node uses one, collect_node adds its defining metadata. */
    uint32_t index = ly_ctx_internal_modules_count(ctx);
    const struct lys_module *module;
    while ((module = ly_ctx_get_module_iter(ctx, &index))) {
        if (module->implemented && add_module(&p, module)) { goto cleanup; }
    }
    if (p.module_count > 1) {
        qsort(p.modules, p.module_count, sizeof *p.modules, compare_modules);
    }
    p.row_count = 1; /* Reserve the synthetic root (no libyang pointer). */
    p.row_capacity = 64;
    p.rows = calloc(p.row_capacity, sizeof *p.rows);
    if (!p.rows) { goto cleanup; }
    size_t root_module_count = p.module_count;
    for (size_t i = 0; i < root_module_count; ++i) {
        module = p.modules[i];
        if (!module->compiled) { continue; }
        for (const struct lysc_node *node = module->compiled->data; node; node = node->next) {
            /* Depth includes the virtual root: top-level YANG nodes are 1. */
            if (collect_node(&p, node, 0, 1)) { goto cleanup; }
        }
    }
    if (p.module_count > 1) {
        qsort(p.modules, p.module_count, sizeof *p.modules, compare_modules);
    }

    fputs("{\n  \"format_version\": 1,\n  \"compiler\": ", stdout);
    json_string(COMPILER_NAME);
    fputs(",\n  \"modules\": [\n", stdout);
    for (size_t i = 0; i < p.module_count; ++i) {
        if (i) { fputs(",\n", stdout); }
        if (emit_module(p.modules[i])) { goto cleanup; }
    }
    fputs("\n  ],\n  \"nodes\": [\n"
          "    {\"id\":0,\"parent\":null,\"kind\":\"root\",\"module\":\"\",\"namespace\":\"\",\"name\":\"\","
          "\"config\":null,\"key\":false,\"mandatory\":false,\"presence\":false,\"status\":\"current\","
          "\"type_name\":null,\"when\":[],\"must\":[]}", stdout);
    for (size_t i = 1; i < p.row_count; ++i) {
        fputs(",\n", stdout);
        emit_node(&p.rows[i], (uint32_t)i);
    }
    fputs("\n  ]\n}\n", stdout);
    if (fflush(stdout) || ferror(stdout)) {
        perror("writing schema JSON");
        goto cleanup;
    }
    result = EXIT_SUCCESS;

cleanup:
    if (result != EXIT_SUCCESS) { fputs("schema-export failed.\n", stderr); }
    ly_ctx_destroy(ctx);
    free(p.rows);
    free(p.modules);
    for (size_t i = 0; i < request_count; ++i) { free(requests[i].module); }
    free(requests);
    free(search_dirs);
    free(paths);
    return result;
}
