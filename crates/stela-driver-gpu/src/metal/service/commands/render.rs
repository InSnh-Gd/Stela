//! Commands of `MTLRenderCommandEncoder`.
use super::{Payload, binding, count, depth_stencil_state, grid, index_type, primitive, size};
use crate::metal::{
    abi::{
        BarrierScope, CullMode, DepthClipMode, LoadAction, MultisampleDepthResolveFilter,
        MultisampleStencilResolveFilter, RenderStages, StoreAction, StoreActionOptions,
        TriangleFillMode, VisibilityResultMode, VisibilityResultType, Winding,
    },
    command_buffer::{
        CommandBuffer,
        render::{PassBoundary, ScissorRect, Viewport},
    },
    compiler::DEFERRED,
    device::{
        BufferId, ComputePipelineId, CounterBufferId, RenderPipelineId, TextureId,
        queue::{AttachmentRequest, RenderPassRequest},
    },
    error::Error,
    pass::{AttachmentSlot, TileMemory},
    pipeline::MAX_COLOR_ATTACHMENTS,
    service::{
        Service, boolean, decode, decode_bits,
        pipelines::render_stage,
        wire::{
            self, Fault, Status,
            commands::{
                BeginRender, BlendColor, ColorStoreAction, ColorStoreActionOptions,
                CounterSampleRecord, DepthBias, DrawIndexed, DrawIndexedIndirect, DrawIndirect,
                DrawMesh, DrawMeshIndirect, DrawPatches, DrawPatchesIndirect, DrawPrimitives,
                FactorBuffer, FactorScale, Mode, PassAttachmentRecord, PassBoundaryCode,
                RenderBarrier, RenderCode, ScissorRecord, SetPipeline, SetThreadgroupMemory,
                StageBuffer, StageBufferOffset, StageBytes, StageSampler, StageTexture,
                StencilReference, TileDispatch, TileThreadgroupMemory, VertexBufferStride,
                VertexOffsetStride, ViewMappingRecord, ViewportRecord, VisibilityMode,
            },
        },
    },
    shader::Compiler,
    vulkan::backend::Backend,
};
use alloc::{format, string::String, vec::Vec};
use core::num::NonZeroU64;

/// `MTLRenderStageTile`, whose bindings tile dispatches read.
const TILE_STAGE: u64 = 4;

impl<B: Backend, C: Compiler> Service<B, C> {
    pub(super) fn render(
        &mut self,
        connection: NonZeroU64,
        commands: &mut CommandBuffer,
        code: RenderCode,
        payload: Payload,
    ) -> Result<(), Fault> {
        match code {
            RenderCode::Begin => {
                let (record, data) = payload.split::<BeginRender>()?;
                let request = self.render_pass(connection, &record, data.bytes())?;
                if let Err(error) = self.device.begin_render(commands, &request) {
                    // A deferral is no refusal: the call is answered
                    // `Pending` and the pass begins when it runs again.
                    if error != DEFERRED {
                        self.noted = Some(self.describe_pass(&request, error));
                    }
                    return Err(error.into());
                }
                #[cfg(not(test))]
                {
                    use crate::metal::pixel_format::PixelFormat as P;
                    self.diag_bound_pipeline = None;
                    self.diag_vp = "default".into();
                    self.diag_sc = "default".into();
                    self.diag_cull = "None".into();
                    self.diag_ds = "none".into();
                    self.diag_frag_tex.clear();
                    self.diag_frag_buf.clear();
                    self.diag_pass_c0 = request.color_attachments[0].as_ref().map(|att| {
                        let s = format!(
                            "{} load={:?} store={:?} surf={}",
                            self.device.diag_texture_summary(att.texture),
                            att.load_action,
                            att.store_action,
                            att.surface.is_some()
                        );
                        (att.texture, s)
                    });
                    self.diag_pass_c1 = request.color_attachments[1].as_ref().map(|att| {
                        let s = format!(
                            "{} load={:?} store={:?} surf={}",
                            self.device.diag_texture_summary(att.texture),
                            att.load_action,
                            att.store_action,
                            att.surface.is_some()
                        );
                        (att.texture, s)
                    });
                    let interesting = request.color_attachments.iter().flatten().any(|att| {
                        let fmt = self
                            .device
                            .texture_descriptor(att.texture)
                            .ok()
                            .map(|d| d.pixel_format);
                        att.load_action != LoadAction::Clear
                            || matches!(
                                fmt,
                                Some(
                                    P::Bgra10Xr
                                        | P::Bgra10XrSrgb
                                        | P::Rgba16Float
                                        | P::R16Float
                                        | P::Rgb10A8_2pXr10
                                        | P::Rgb10A8_2pXr10Srgb
                                )
                            )
                    });
                    if interesting && self.diag_draw_count < 90 {
                        let mut msg = String::from("DIAG PASS");
                        for (index, att) in request.color_attachments.iter().enumerate() {
                            if let Some(att) = att {
                                let d = self.device.texture_descriptor(att.texture).ok();
                                let _ = core::fmt::Write::write_fmt(
                                    &mut msg,
                                    format_args!(
                                        " c{index}=[{:?} s={} load={:?} store={:?} surf={} res={}]",
                                        d.as_ref().map(|x| x.pixel_format),
                                        d.as_ref().map_or(0, |x| x.sample_count),
                                        att.load_action,
                                        att.store_action,
                                        att.surface.is_some(),
                                        att.resolve.is_some(),
                                    ),
                                );
                            }
                        }
                        self.refused.push(msg);
                    }
                }
            }
            RenderCode::SetPipeline => {
                let SetPipeline { pipeline } = payload.record()?;
                let pipeline = self.object::<RenderPipelineId>(connection, pipeline)?;
                #[cfg(not(test))]
                {
                    self.diag_bound_pipeline = Some(pipeline.0);
                }
                commands.set_render_pipeline_state(self.device.render_pipeline_state(pipeline)?)?;
            }
            RenderCode::SetBuffer => {
                let StageBuffer {
                    stage,
                    buffer,
                    offset,
                    index,
                } = payload.record()?;
                let buffer = self.optional_buffer(connection, buffer)?;
                #[cfg(not(test))]
                if stage != TILE_STAGE && matches!(render_stage(stage), Ok(crate::metal::shader::Stage::Fragment)) {
                    self.diag_frag_buf.insert(index as u32, format!("buf({buffer:?}+0x{offset:x})"));
                }
                if stage == TILE_STAGE {
                    commands.set_tile_buffer(buffer, offset, binding(index)?)?;
                } else {
                    commands.set_render_buffer(
                        render_stage(stage)?,
                        buffer,
                        offset,
                        binding(index)?,
                    )?;
                }
            }
            RenderCode::SetBufferOffset => {
                let StageBufferOffset {
                    stage,
                    offset,
                    index,
                } = payload.record()?;
                if stage == TILE_STAGE {
                    commands.set_tile_buffer_offset(offset, binding(index)?)?;
                } else {
                    commands.set_render_buffer_offset(
                        render_stage(stage)?,
                        offset,
                        binding(index)?,
                    )?;
                }
            }
            RenderCode::SetBytes => {
                let (StageBytes { stage, index }, bytes) = payload.split()?;
                #[cfg(not(test))]
                if stage != TILE_STAGE && matches!(render_stage(stage), Ok(crate::metal::shader::Stage::Fragment)) {
                    let b = bytes.bytes();
                    let mut hex = format!("bytes(len={}:", b.len());
                    for byte in b.iter().take(32) {
                        let _ = core::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
                    }
                    hex.push(')');
                    self.diag_frag_buf.insert(index as u32, hex);
                }
                let resolver = &mut self.device.resolver();
                if stage == TILE_STAGE {
                    commands.set_tile_bytes(resolver, bytes.bytes(), binding(index)?)?;
                } else {
                    commands.set_render_bytes(
                        resolver,
                        render_stage(stage)?,
                        bytes.bytes(),
                        binding(index)?,
                    )?;
                }
            }
            RenderCode::SetTexture => {
                let StageTexture {
                    stage,
                    texture,
                    index,
                } = payload.record()?;
                self.note_bound(connection, commands, texture)?;
                if texture != 0 {
                    let id = self.object::<TextureId>(connection, texture)?;
                    self.device.note_sampled(id);
                    #[cfg(not(test))]
                    if let Ok(crate::metal::shader::Stage::Fragment) = render_stage(stage) {
                        self.diag_frag_tex.insert(index as u32, id);
                    }
                } else {
                    #[cfg(not(test))]
                    if let Ok(crate::metal::shader::Stage::Fragment) = render_stage(stage) {
                        self.diag_frag_tex.remove(&(index as u32));
                    }
                }
                let texture = self.texture_id(connection, texture)?;
                if stage == TILE_STAGE {
                    commands.set_tile_texture(texture, binding(index)?)?;
                } else {
                    commands.set_render_texture(render_stage(stage)?, texture, binding(index)?)?;
                }
            }
            RenderCode::SetSampler => {
                let request: StageSampler = payload.record()?;
                let sampler = self.sampler_id(connection, request.sampler)?;
                let clamp = boolean(request.clamped)?.then_some((request.lod_min, request.lod_max));
                let resolver = &mut self.device.resolver();
                if request.stage == TILE_STAGE {
                    commands.set_tile_sampler(resolver, sampler, clamp, binding(request.index)?)?;
                } else {
                    commands.set_render_sampler(
                        resolver,
                        render_stage(request.stage)?,
                        sampler,
                        clamp,
                        binding(request.index)?,
                    )?;
                }
            }
            RenderCode::SetVertexBufferStride => {
                let request: VertexBufferStride = payload.record()?;
                let buffer = self.buffer_ref(connection, request.buffer)?;
                commands.set_vertex_buffer_with_stride(
                    buffer,
                    request.offset,
                    request.stride,
                    binding(request.index)?,
                )?;
            }
            RenderCode::SetVertexBufferOffsetStride => {
                let request: VertexOffsetStride = payload.record()?;
                commands.set_vertex_buffer_offset_with_stride(
                    request.offset,
                    request.stride,
                    binding(request.index)?,
                )?;
            }
            RenderCode::SetViewports => {
                let viewports: Vec<Viewport> = payload
                    .list::<ViewportRecord>()?
                    .into_iter()
                    .map(|record| Viewport {
                        origin_x: record.origin_x,
                        origin_y: record.origin_y,
                        width: record.width,
                        height: record.height,
                        znear: record.znear,
                        zfar: record.zfar,
                    })
                    .collect();
                #[cfg(not(test))]
                {
                    self.diag_vp = viewports
                        .first()
                        .map_or_else(|| "-".into(), |v| format!("{:.0},{:.0} {}x{}", v.origin_x, v.origin_y, v.width, v.height));
                }
                commands.set_viewports(&viewports)?;
            }
            RenderCode::SetScissorRects => {
                let rects: Vec<ScissorRect> = payload
                    .list::<ScissorRecord>()?
                    .into_iter()
                    .map(|record| ScissorRect {
                        x: record.x,
                        y: record.y,
                        width: record.width,
                        height: record.height,
                    })
                    .collect();
                #[cfg(not(test))]
                {
                    self.diag_sc = rects
                        .first()
                        .map_or_else(|| "-".into(), |s| format!("{},{} {}x{}", s.x, s.y, s.width, s.height));
                }
                commands.set_scissor_rects(&rects)?;
            }
            RenderCode::SetCullMode => {
                let Mode { mode } = payload.record()?;
                let cull = decode(CullMode::from_raw, mode, "cull mode")?;
                #[cfg(not(test))]
                {
                    self.diag_cull = format!("{cull:?}");
                }
                commands.set_cull_mode(cull)?;
            }
            RenderCode::SetFrontFacingWinding => {
                let Mode { mode } = payload.record()?;
                commands.set_front_facing_winding(decode(Winding::from_raw, mode, "winding")?)?;
            }
            RenderCode::SetDepthBias => {
                let DepthBias {
                    constant,
                    slope,
                    clamp,
                    reserved: _,
                } = payload.record()?;
                commands.set_depth_bias(constant, slope, clamp)?;
            }
            RenderCode::SetDepthStencilState => {
                let state = depth_stencil_state(&payload.record()?)?;
                #[cfg(not(test))]
                {
                    self.diag_ds = format!("dw={} dc={:?}", state.depth_write_enable, state.depth_compare_op);
                }
                commands.set_depth_stencil_state(Some(&state))?;
            }
            RenderCode::SetTilePipeline => {
                let SetPipeline { pipeline } = payload.record()?;
                let pipeline = self.object::<ComputePipelineId>(connection, pipeline)?;
                let (state, target) = self.device.tile_pipeline_state(pipeline)?;
                commands.set_tile_pipeline_state(state, target)?;
            }
            RenderCode::SetTileThreadgroupMemory => {
                let TileThreadgroupMemory {
                    length,
                    offset,
                    index,
                } = payload.record()?;
                commands.set_tile_threadgroup_memory(length, offset, binding(index)?)?;
            }
            RenderCode::DispatchThreadsPerTile => {
                let TileDispatch { threads } = payload.record()?;
                commands.dispatch_threads_per_tile(&mut self.device.resolver(), size(threads))?;
            }
            RenderCode::SetObjectThreadgroupMemory => {
                let SetThreadgroupMemory { length, index } = payload.record()?;
                commands.set_object_threadgroup_memory_length(length, binding(index)?)?;
            }
            RenderCode::SetStencilReference => {
                let StencilReference { front, back } = payload.record()?;
                commands.set_stencil_reference_values(front, back)?;
            }
            RenderCode::SetBlendColor => {
                let BlendColor { color } = payload.record()?;
                commands.set_blend_color(color)?;
            }
            RenderCode::SetTriangleFillMode => {
                let Mode { mode } = payload.record()?;
                let mode = decode(TriangleFillMode::from_raw, mode, "triangle fill mode")?;
                commands.set_triangle_fill_mode(mode)?;
            }
            RenderCode::SetDepthClipMode => {
                let Mode { mode } = payload.record()?;
                commands.set_depth_clip_mode(decode(
                    DepthClipMode::from_raw,
                    mode,
                    "clip mode",
                )?)?;
            }
            RenderCode::SetVertexAmplification => {
                let mappings: Vec<(u32, u32)> = payload
                    .list::<ViewMappingRecord>()?
                    .into_iter()
                    .map(|mapping| {
                        (
                            mapping.viewport_array_index_offset,
                            mapping.render_target_array_index_offset,
                        )
                    })
                    .collect();
                commands.set_vertex_amplification(&mappings)?;
            }
            RenderCode::SetVisibilityResultMode => {
                let VisibilityMode { mode, offset } = payload.record()?;
                let mode = decode(VisibilityResultMode::from_raw, mode, "visibility mode")?;
                commands.set_visibility_result_mode(mode, offset)?;
            }
            RenderCode::SetColorStoreAction => {
                let ColorStoreAction { action, index } = payload.record()?;
                let index = usize::try_from(index)
                    .map_err(|_| Fault::malformed("color index").value(index))?;
                commands.set_store_action(AttachmentSlot::Color(index), store_action(action)?)?;
            }
            RenderCode::SetDepthStoreAction => {
                let Mode { mode } = payload.record()?;
                commands.set_store_action(AttachmentSlot::Depth, store_action(mode)?)?;
            }
            RenderCode::SetStencilStoreAction => {
                let Mode { mode } = payload.record()?;
                commands.set_store_action(AttachmentSlot::Stencil, store_action(mode)?)?;
            }
            RenderCode::SetColorStoreActionOptions => {
                let ColorStoreActionOptions { options, index } = payload.record()?;
                let index = usize::try_from(index)
                    .map_err(|_| Fault::malformed("color index").value(index))?;
                commands.set_store_action_options(
                    AttachmentSlot::Color(index),
                    store_options(options)?,
                )?;
            }
            RenderCode::SetDepthStoreActionOptions => {
                let Mode { mode } = payload.record()?;
                commands.set_store_action_options(AttachmentSlot::Depth, store_options(mode)?)?;
            }
            RenderCode::SetStencilStoreActionOptions => {
                let Mode { mode } = payload.record()?;
                commands.set_store_action_options(AttachmentSlot::Stencil, store_options(mode)?)?;
            }
            RenderCode::MemoryBarrier => {
                let RenderBarrier {
                    scope,
                    after,
                    before,
                } = payload.record()?;
                let scope = decode_bits(BarrierScope::from_bits, scope, "barrier scope")?;
                let stages = |raw| decode_bits(RenderStages::from_bits, raw, "render stages");
                commands.render_memory_barrier(
                    &mut self.device.resolver(),
                    scope,
                    stages(after)?,
                    stages(before)?,
                )?;
            }
            RenderCode::DrawPrimitives => {
                let draw: DrawPrimitives = payload.record()?;
                #[cfg(not(test))]
                self.diag_note_draw("prim", draw.vertex_count, draw.instance_count);
                commands.draw_primitives(
                    &mut self.device.resolver(),
                    primitive(draw.primitive)?,
                    count(draw.vertex_start)?,
                    count(draw.vertex_count)?,
                    count(draw.instance_count)?,
                    count(draw.base_instance)?,
                )?;
            }
            RenderCode::DrawIndexed => {
                let draw: DrawIndexed = payload.record()?;
                #[cfg(not(test))]
                self.diag_note_draw("idx", draw.index_count, draw.instance_count);
                let index_buffer = self.buffer_ref(connection, draw.index_buffer)?;
                let base_vertex = i32::try_from(draw.base_vertex).map_err(|_| {
                    Fault::new(Status::Invalid, "the base vertex exceeds 32 bits")
                        .signed(draw.base_vertex)
                })?;
                commands.draw_indexed_primitives(
                    &mut self.device.resolver(),
                    primitive(draw.primitive)?,
                    count(draw.index_count)?,
                    index_type(draw.index_type)?,
                    index_buffer,
                    draw.index_buffer_offset,
                    count(draw.instance_count)?,
                    base_vertex,
                    count(draw.base_instance)?,
                )?;
            }
            RenderCode::DrawIndirect => {
                let draw: DrawIndirect = payload.record()?;
                let indirect = self.buffer_ref(connection, draw.buffer)?;
                commands.draw_primitives_indirect(
                    &mut self.device.resolver(),
                    primitive(draw.primitive)?,
                    indirect,
                    draw.offset,
                )?;
            }
            RenderCode::DrawIndexedIndirect => {
                let draw: DrawIndexedIndirect = payload.record()?;
                let index_buffer = self.buffer_ref(connection, draw.index_buffer)?;
                let indirect = self.buffer_ref(connection, draw.buffer)?;
                commands.draw_indexed_primitives_indirect(
                    &mut self.device.resolver(),
                    primitive(draw.primitive)?,
                    index_type(draw.index_type)?,
                    index_buffer,
                    draw.index_buffer_offset,
                    indirect,
                    draw.offset,
                )?;
            }
            RenderCode::SetTessellationFactorBuffer => {
                let FactorBuffer {
                    buffer,
                    offset,
                    instance_stride,
                } = payload.record()?;
                let buffer = self.optional_buffer(connection, buffer)?;
                commands.set_tessellation_factor_buffer(buffer, offset, instance_stride)?;
            }
            RenderCode::SetTessellationFactorScale => {
                let FactorScale { scale, reserved: _ } = payload.record()?;
                commands.set_tessellation_factor_scale(scale)?;
            }
            RenderCode::DrawPatches => {
                let draw: DrawPatches = payload.record()?;
                let source = self.patch_source(connection, &draw.source)?;
                commands.draw_patches(
                    &mut self.device.resolver(),
                    &source,
                    count(draw.patch_start)?,
                    count(draw.patch_count)?,
                    count(draw.instance_count)?,
                    count(draw.base_instance)?,
                )?;
            }
            RenderCode::DrawPatchesIndirect => {
                let draw: DrawPatchesIndirect = payload.record()?;
                let source = self.patch_source(connection, &draw.source)?;
                let indirect = self.buffer_ref(connection, draw.buffer)?;
                commands.draw_patches_indirect(
                    &mut self.device.resolver(),
                    &source,
                    indirect,
                    draw.offset,
                )?;
            }
            RenderCode::DrawMeshThreadgroups => {
                let draw: DrawMesh = payload.record()?;
                commands.draw_mesh_threadgroups(
                    &mut self.device.resolver(),
                    grid(draw.size)?,
                    grid(draw.object_threads)?,
                    grid(draw.mesh_threads)?,
                )?;
            }
            RenderCode::DrawMeshThreads => {
                let draw: DrawMesh = payload.record()?;
                commands.draw_mesh_threads(
                    &mut self.device.resolver(),
                    grid(draw.size)?,
                    grid(draw.object_threads)?,
                    grid(draw.mesh_threads)?,
                )?;
            }
            RenderCode::DrawMeshIndirect => {
                let draw: DrawMeshIndirect = payload.record()?;
                let indirect = self.buffer_ref(connection, draw.buffer)?;
                commands.draw_mesh_threadgroups_indirect(
                    &mut self.device.resolver(),
                    indirect,
                    draw.offset,
                    grid(draw.object_threads)?,
                    grid(draw.mesh_threads)?,
                )?;
            }
        }
        Ok(())
    }

    /// The `MTLRenderPassDescriptor` of `record` and its data: sample
    /// positions, then counter samples.
    fn render_pass(
        &self,
        connection: NonZeroU64,
        record: &BeginRender,
        data: &[u8],
    ) -> Result<RenderPassRequest, Fault> {
        let positions = record.sample_position_count as usize * size_of::<[f32; 2]>();
        let (positions, samples) = data.split_at_checked(positions).ok_or(Fault::MALFORMED)?;
        let samples = wire::read_array::<CounterSampleRecord>(samples)?;
        if samples.len() != record.counter_sample_count as usize {
            return Err(Fault::MALFORMED);
        }
        let mut color_attachments = [None; MAX_COLOR_ATTACHMENTS];
        for (slot, color) in color_attachments.iter_mut().zip(&record.color_attachments) {
            *slot = self.attachment(connection, &color.attachment, color.clear_color)?;
        }
        let (depth, stencil) = (&record.depth_attachment, &record.stencil_attachment);
        Ok(RenderPassRequest {
            color_attachments,
            depth_attachment: self.attachment(connection, &depth.attachment, depth.clear_depth)?,
            depth_resolve_filter: decode(
                MultisampleDepthResolveFilter::from_raw,
                depth.resolve_filter,
                "depth resolve filter",
            )?,
            stencil_attachment: self.attachment(
                connection,
                &stencil.attachment,
                stencil.clear_stencil,
            )?,
            stencil_resolve_filter: decode(
                MultisampleStencilResolveFilter::from_raw,
                stencil.resolve_filter,
                "stencil resolve filter",
            )?,
            visibility_result_buffer: (record.visibility_result_buffer != 0)
                .then(|| self.object::<BufferId>(connection, record.visibility_result_buffer))
                .transpose()?,
            visibility_result_type: decode(
                VisibilityResultType::from_raw,
                record.visibility_result_type,
                "visibility result type",
            )?,
            render_target_array_length: record.render_target_array_length,
            render_target_width: record.render_target_width,
            render_target_height: record.render_target_height,
            default_raster_sample_count: record.default_raster_sample_count,
            tile_memory: TileMemory {
                width: record.tile_width,
                height: record.tile_height,
                threadgroup_memory_length: record.threadgroup_memory_length,
                imageblock_sample_length: record.imageblock_sample_length,
            },
            sample_positions: wire::read_array(positions)?,
            rasterization_rate_map: boolean(record.rasterization_rate_map)?,
            counter_samples: samples
                .iter()
                .map(|sample| self.counter_sample(connection, sample))
                .collect::<Result<_, _>>()?,
        })
    }

    /// A refused pass, for the platform's log: why, its color attachments
    /// (each texture and resolve texture as it was made, and the actions)
    /// and its tile memory where the pass sets any.
    fn describe_pass(&self, request: &RenderPassRequest, error: Error) -> String {
        use core::fmt::Write;
        let texture = |id: TextureId| {
            self.device.texture_descriptor(id).map_or_else(
                |_| String::from("?"),
                |d| {
                    format!(
                        "{:?} {}x{}x{} samples={} storage={:?} usage={:?}",
                        d.pixel_format,
                        d.width,
                        d.height,
                        d.depth,
                        d.sample_count,
                        d.options.storage_mode,
                        d.usage
                    )
                },
            )
        };
        let mut text = format!("PASS REFUSED ({})", Fault::from(error));
        for (index, attachment) in request.color_attachments.iter().enumerate() {
            let Some(attachment) = attachment else {
                continue;
            };
            let _ = write!(
                text,
                " color{index}=[{} load={:?} store={:?}",
                texture(attachment.texture),
                attachment.load_action,
                attachment.store_action
            );
            if let Some((resolve, ..)) = attachment.resolve {
                let _ = write!(text, " resolve={}", texture(resolve));
            }
            text.push(']');
        }
        let tile = request.tile_memory;
        if tile != TileMemory::default() {
            let _ = write!(
                text,
                " tile={}x{} threadgroup={} imageblock={}",
                tile.width,
                tile.height,
                tile.threadgroup_memory_length,
                tile.imageblock_sample_length
            );
        }
        text
    }

    /// An attachment of a pass, or none without a texture.
    fn attachment<Clear>(
        &self,
        connection: NonZeroU64,
        record: &PassAttachmentRecord,
        clear: Clear,
    ) -> Result<Option<AttachmentRequest<Clear>>, Fault> {
        if record.texture == 0 {
            return Ok(None);
        }
        let resolve = (record.resolve_texture != 0)
            .then(|| {
                Ok::<_, Fault>((
                    self.object::<TextureId>(connection, record.resolve_texture)?,
                    record.resolve_level,
                    record.resolve_slice,
                    record.resolve_depth_plane,
                ))
            })
            .transpose()?;
        Ok(Some(AttachmentRequest {
            texture: self.object::<TextureId>(connection, record.texture)?,
            level: record.level,
            slice: record.slice,
            depth_plane: record.depth_plane,
            resolve,
            load_action: decode(LoadAction::from_raw, record.load_action, "load action")?,
            store_action: store_action(record.store_action)?,
            store_action_options: store_options(record.store_action_options)?,
            clear,
            surface: (record.surface != 0).then_some((record.surface, record.seed)),
        }))
    }

    fn counter_sample(
        &self,
        connection: NonZeroU64,
        record: &CounterSampleRecord,
    ) -> Result<(CounterBufferId, u32, PassBoundary), Fault> {
        let boundary = match decode(PassBoundaryCode::from_raw, record.boundary, "pass boundary")? {
            PassBoundaryCode::StartOfVertex => PassBoundary::StartOfVertex,
            PassBoundaryCode::EndOfVertex => PassBoundary::EndOfVertex,
            PassBoundaryCode::StartOfFragment => PassBoundary::StartOfFragment,
            PassBoundaryCode::EndOfFragment => PassBoundary::EndOfFragment,
        };
        let index = u32::try_from(record.index)
            .map_err(|_| Fault::malformed("sample index").value(record.index))?;
        let buffer = self.object::<CounterBufferId>(connection, record.buffer)?;
        Ok((buffer, index, boundary))
    }
}

fn store_action(raw: u64) -> Result<StoreAction, Fault> {
    decode(StoreAction::from_raw, raw, "store action")
}

fn store_options(raw: u64) -> Result<StoreActionOptions, Fault> {
    decode_bits(StoreActionOptions::from_bits, raw, "store action options")
}

#[cfg(not(test))]
static DIAG_WS_SEEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(not(test))]
static DIAG_WS_DRAWS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
#[cfg(not(test))]
static DIAG_WS_PROBES: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[cfg(not(test))]
impl<B: Backend, C: Compiler> Service<B, C> {
    fn diag_note_draw(&mut self, kind: &str, verts: u64, insts: u64) {
        use core::sync::atomic::Ordering;
        let Some(pid) = self.diag_bound_pipeline else {
            return;
        };
        let Some((vert, frag, cinfo)) = self.diag_pipelines.get(&pid).cloned() else {
            return;
        };
        let is_blur_or_glass = frag.contains("downsample")
            || frag.contains("narrow_blur")
            || frag.contains("variable_blur")
            || frag.contains("glass_background");
        let is_ws_trigger = is_blur_or_glass || frag.contains("attachment_clear");
        if is_ws_trigger {
            DIAG_WS_SEEN.store(true, Ordering::Relaxed);
        }
        let ws_seen = DIAG_WS_SEEN.load(Ordering::Relaxed);
        let is_rb = frag.contains("uber_fragment")
            || frag.contains("filter_blur_fragment")
            || frag.contains("primitive_image_fragment");
        let is_c0_1280 = self
            .diag_pass_c0
            .as_ref()
            .is_some_and(|x| x.1.contains("1280x800"));
        let is_large_ws = is_c0_1280 && (!self.diag_frag_tex.is_empty() || frag.contains("attachment_clear"));
        let interesting = is_blur_or_glass
            || (is_rb && self.diag_draw_count < 20)
            || (is_large_ws && (self.diag_draw_count < 10 || ws_seen));
        if !interesting {
            return;
        }
        let is_wall_or_glass = self.diag_frag_tex.values().any(|&id| {
            self.device.texture_descriptor(id).is_ok_and(|d| d.width >= 2000)
        }) || frag.contains("glass_background");
        static DIAG_WALL_DRAWS: core::sync::atomic::AtomicUsize =
            core::sync::atomic::AtomicUsize::new(0);
        let log_ws = (ws_seen && DIAG_WS_DRAWS.load(Ordering::Relaxed) < 200)
            || (is_wall_or_glass && DIAG_WALL_DRAWS.load(Ordering::Relaxed) < 30);
        if self.diag_draw_count < 10 || log_ws {
            if self.diag_draw_count < 10 {
                self.diag_draw_count += 1;
            }
            let (prefix, seq) = if is_wall_or_glass && DIAG_WS_DRAWS.load(Ordering::Relaxed) >= 200 {
                ("WALL", DIAG_WALL_DRAWS.fetch_add(1, Ordering::Relaxed) + 1)
            } else if ws_seen {
                ("WSDRAW", DIAG_WS_DRAWS.fetch_add(1, Ordering::Relaxed) + 1)
            } else {
                ("DRAW", self.diag_draw_count as usize)
            };
            let mut msg = format!(
                "DIAG {prefix} #{seq} {vert}/{frag}[{cinfo}] vp=[{}] sc=[{}] cull=[{}] ds=[{}] {kind}({verts}x{insts}) c0=[{}] c1=[{}] tex=[",
                self.diag_vp,
                self.diag_sc,
                self.diag_cull,
                self.diag_ds,
                self.diag_pass_c0.as_ref().map_or("-", |x| x.1.as_str()),
                self.diag_pass_c1.as_ref().map_or("-", |x| x.1.as_str()),
            );
            for (slot, id) in &self.diag_frag_tex {
                let _ = core::fmt::Write::write_fmt(
                    &mut msg,
                    format_args!(" {slot}:{}", self.device.diag_texture_summary(*id)),
                );
            }
            msg.push_str(" ] fbuf=[");
            for (slot, summary) in &self.diag_frag_buf {
                let _ = core::fmt::Write::write_fmt(
                    &mut msg,
                    format_args!(" {slot}:{summary}"),
                );
            }
            msg.push_str(" ]");
            self.refused.push(msg);
        }
        let tex_src = self
            .diag_frag_tex
            .get(&3)
            .or_else(|| self.diag_frag_tex.get(&0))
            .or_else(|| self.diag_frag_tex.values().next())
            .copied();
        let tex_desc = tex_src.and_then(|id| self.device.texture_descriptor(id).ok());
        let tex_w = tex_desc.map_or(0, |d| d.width);
        let tex_h = tex_desc.map_or(0, |d| d.height);
        let is_wall = tex_w >= 2000;
        if is_wall {
            self.diag_wall_seen = true;
        }
        let tex_src4 = self
            .diag_frag_tex
            .get(&4)
            .copied()
            .filter(|&id| {
                vert == "fixed_vert_lph_gen"
                    || self.device
                        .texture_descriptor(id)
                        .ok()
                        .is_some_and(|d| d.width > 1 || d.height > 1)
            });
        let post_wall = self.diag_wall_seen
            && self.diag_wall_probes < 40
            && (is_wall || is_blur_or_glass || is_large_ws)
            && !(tex_w == 1 && tex_h == 1);
        let ws_probe = ws_seen && is_c0_1280 && DIAG_WS_PROBES.load(Ordering::Relaxed) < 25;
        let is_glass = frag.contains("glass_background");
        let early_probe = (self.diag_probe_count < 16
            && (frag.contains("downsample_4_frag")
                || frag.contains("downsample_8_frag")
                || is_glass
                || frag.contains("variable_blur_downsample")))
            || (is_glass && self.diag_probe_count < 24);
        if post_wall {
            self.diag_wall_probes += 1;
            self.diag_probe_queue.push((
                format!("W#{}:{}", self.diag_wall_probes, frag),
                tex_src,
                tex_src4,
                self.diag_pass_c0.as_ref().map(|x| x.0),
            ));
        } else if ws_probe {
            let n = DIAG_WS_PROBES.fetch_add(1, Ordering::Relaxed) + 1;
            self.diag_probe_queue.push((
                format!("WS#{n}:{frag}"),
                tex_src,
                tex_src4,
                self.diag_pass_c0.as_ref().map(|x| x.0),
            ));
            if vert == "fixed_vert_lph_gen" && n <= 8 {
                self.diag_probe_queue.push((
                    format!("WS#{n}:AUX9_12"),
                    self.diag_frag_tex.get(&9).copied(),
                    self.diag_frag_tex.get(&12).copied(),
                    None,
                ));
            }
        } else if early_probe {
            self.diag_probe_count += 1;
            self.diag_probe_queue.push((
                format!("#{}:{}", self.diag_probe_count, frag),
                tex_src,
                tex_src4,
                self.diag_pass_c0.as_ref().map(|x| x.0),
            ));
        }
    }
}
