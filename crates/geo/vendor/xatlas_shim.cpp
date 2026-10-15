// C ABI over xatlas for engine-geo (feature `uv`). Written for this crate (MIT, same as the
// engine); xatlas itself is vendored unmodified beside it (see vendor/README.md).
//
// One call runs the whole pipeline on one mesh and keeps the result behind an opaque handle,
// so the Rust side never touches a C++ struct layout.
#include "xatlas/xatlas.h"

#include <stdint.h>

extern "C" {

struct GeoXatlasParams {
    float max_chart_area;
    float max_boundary_length;
    float normal_deviation_weight;
    float roundness_weight;
    float straightness_weight;
    float normal_seam_weight;
    float texture_seam_weight;
    float max_cost;
    uint32_t max_iterations;
    uint32_t use_input_uvs;
    uint32_t fix_winding;
    uint32_t max_chart_size;
    uint32_t padding;
    float texels_per_unit;
    uint32_t resolution;
    uint32_t bilinear;
    uint32_t block_align;
    uint32_t brute_force;
    uint32_t rotate_charts_to_axis;
    uint32_t rotate_charts;
};

struct GeoXatlasInfo {
    uint32_t width;
    uint32_t height;
    uint32_t chart_count;
    uint32_t vertex_count;
    uint32_t index_count;
    float texels_per_unit;
};

// Returns an atlas handle, or null with *error_code set (xatlas::AddMeshError value).
void *geo_xatlas_generate(const float *positions, const float *normals, const float *uvs,
                          uint32_t vertex_count, const uint32_t *indices, uint32_t index_count,
                          const GeoXatlasParams *p, int32_t *error_code, GeoXatlasInfo *info) {
    xatlas::Atlas *atlas = xatlas::Create();
    xatlas::MeshDecl decl;
    decl.vertexPositionData = positions;
    decl.vertexPositionStride = sizeof(float) * 3;
    if (normals) {
        decl.vertexNormalData = normals;
        decl.vertexNormalStride = sizeof(float) * 3;
    }
    if (uvs) {
        decl.vertexUvData = uvs;
        decl.vertexUvStride = sizeof(float) * 2;
    }
    decl.vertexCount = vertex_count;
    decl.indexData = indices;
    decl.indexCount = index_count;
    decl.indexFormat = xatlas::IndexFormat::UInt32;
    xatlas::AddMeshError err = xatlas::AddMesh(atlas, decl);
    if (err != xatlas::AddMeshError::Success) {
        *error_code = (int32_t)err;
        xatlas::Destroy(atlas);
        return nullptr;
    }
    xatlas::AddMeshJoin(atlas);
    xatlas::ChartOptions co;
    co.maxChartArea = p->max_chart_area;
    co.maxBoundaryLength = p->max_boundary_length;
    co.normalDeviationWeight = p->normal_deviation_weight;
    co.roundnessWeight = p->roundness_weight;
    co.straightnessWeight = p->straightness_weight;
    co.normalSeamWeight = p->normal_seam_weight;
    co.textureSeamWeight = p->texture_seam_weight;
    co.maxCost = p->max_cost;
    co.maxIterations = p->max_iterations;
    co.useInputMeshUvs = p->use_input_uvs != 0;
    co.fixWinding = p->fix_winding != 0;
    xatlas::PackOptions po;
    po.maxChartSize = p->max_chart_size;
    po.padding = p->padding;
    po.texelsPerUnit = p->texels_per_unit;
    po.resolution = p->resolution;
    po.bilinear = p->bilinear != 0;
    po.blockAlign = p->block_align != 0;
    po.bruteForce = p->brute_force != 0;
    po.createImage = false;
    po.rotateChartsToAxis = p->rotate_charts_to_axis != 0;
    po.rotateCharts = p->rotate_charts != 0;
    xatlas::Generate(atlas, co, po);
    if (atlas->meshCount != 1) {
        *error_code = -1;
        xatlas::Destroy(atlas);
        return nullptr;
    }
    const xatlas::Mesh &m = atlas->meshes[0];
    info->width = atlas->width;
    info->height = atlas->height;
    info->chart_count = atlas->chartCount;
    info->vertex_count = m.vertexCount;
    info->index_count = m.indexCount;
    info->texels_per_unit = atlas->texelsPerUnit;
    *error_code = 0;
    return atlas;
}

// Copies the output mesh: per output vertex the source vertex (xref), UV in texels and the
// chart index (-1 when none), and the new index buffer.
void geo_xatlas_copy(const void *handle, uint32_t *xref, float *uv, int32_t *chart,
                     uint32_t *indices) {
    const xatlas::Atlas *atlas = (const xatlas::Atlas *)handle;
    const xatlas::Mesh &m = atlas->meshes[0];
    for (uint32_t i = 0; i < m.vertexCount; i++) {
        xref[i] = m.vertexArray[i].xref;
        uv[2 * i] = m.vertexArray[i].uv[0];
        uv[2 * i + 1] = m.vertexArray[i].uv[1];
        chart[i] = m.vertexArray[i].chartIndex;
    }
    for (uint32_t i = 0; i < m.indexCount; i++)
        indices[i] = m.indexArray[i];
}

void geo_xatlas_destroy(void *handle) { xatlas::Destroy((xatlas::Atlas *)handle); }

} // extern "C"
