import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../core/providers.dart';
import 'force_layout.dart';
import 'graph_painter.dart';

/// Interactive graph widget.
class GraphWidget extends ConsumerStatefulWidget {
  const GraphWidget({super.key});

  @override
  ConsumerState<GraphWidget> createState() => _GraphWidgetState();
}

class _GraphWidgetState extends ConsumerState<GraphWidget>
    with TickerProviderStateMixin {
  late Ticker _ticker;
  
  // Camera Animation for Spotlight
  late AnimationController _cameraAnimController;
  Offset _startOffset = Offset.zero;
  Offset _targetOffset = Offset.zero;
  double _startScale = 1.0;
  double _targetScale = 1.0;
  
  // Viewport transformation
  Offset _offset = Offset.zero;
  double _scale = 1.0;
  double _baseScale = 1.0;
  double _interactionSpeed = 0.0;
  
  // Interaction
  String? _hoveredNodeId;
  GraphNode? _draggedNode;
  bool _hasCentered = false;
  String? _lastSpotlightId;

  @override
  void initState() {
    super.initState();
    
    // Camera animation controller for smooth spotlight transitions
    _cameraAnimController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 600),
    )..addListener(() {
      // Interpolate camera during animation
      setState(() {
        final t = Curves.easeOutCubic.transform(_cameraAnimController.value);
        _scale = _startScale + (_targetScale - _startScale) * t;
        _offset = Offset(
          _startOffset.dx + (_targetOffset.dx - _startOffset.dx) * t,
          _startOffset.dy + (_targetOffset.dy - _startOffset.dy) * t,
        );
      });
    });
    
    // Setup physics loop
    _ticker = createTicker((elapsed) {
      final state = ref.read(graphProvider);
      
      // Run physics simulation
      if (state.nodes.isNotEmpty) {
        // Use fixed delta time for stability
        final stillMoving = ForceLayout.update(
          state.nodes, 
          state.edges, 
          0.016 // ~60 FPS
        );
        
        // Always repaint to animate movement
        setState(() {}); 
      }
    });
    
    _ticker.start();
    
    // Connect to server on startup
    WidgetsBinding.instance.addPostFrameCallback((_) {
      ref.read(graphProvider.notifier).connect();
    });
  }

  @override
  void dispose() {
    _ticker.dispose();
    _cameraAnimController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final state = ref.watch(graphProvider);

    // Auto-center "Drone Shot"
    if (!_hasCentered && state.nodes.isNotEmpty) {
      _hasCentered = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
         final size = MediaQuery.of(context).size;
         _zoomToFit(state.nodes, size);
      });
    }
    
    // Spotlight Tracking: AI Focus Camera Animation
    if (state.isFollowMode && state.spotlightNodeId != null && state.spotlightNodeId != _lastSpotlightId) {
      _lastSpotlightId = state.spotlightNodeId;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        _animateToNode(state.spotlightNodeId!, state.nodes, MediaQuery.of(context).size);
      });
    }
    
    return Stack(
      children: [
        GestureDetector(
          onScaleStart: (details) {
            final localPos = details.localFocalPoint;
            final node = _hitTest(localPos, state.nodes);
            if (node != null) {
              _draggedNode = node;
              HapticFeedback.lightImpact();
            } else {
              _baseScale = _scale;
            }
          },
          onScaleUpdate: (details) {
            setState(() {
              if (_draggedNode != null) {
                // Drag Influence: Move node and reset velocity
                final delta = details.focalPointDelta / _scale;
                _draggedNode!.x += delta.dx;
                _draggedNode!.y += delta.dy;
                _draggedNode!.vx = 0;
                _draggedNode!.vy = 0;
              } else {
                // Viewport Pan & Zoom
                _offset += details.focalPointDelta;
                _scale = (_baseScale * details.scale).clamp(0.1, 5.0);
                
                // Calculate interaction speed for LOD
                // We use the focal point movement as a proxy for speed.
                // For pure zooming, we might want to consider scale delta too, 
                // but usually zooming involves some focal shift or we can just rely on pan.
                _interactionSpeed = details.focalPointDelta.distance;
              }
            });
          },
          onScaleEnd: (details) {
            _draggedNode = null;
            setState(() {
              _interactionSpeed = 0.0;
            });
          },
          onTapUp: (details) => _handleTap(details, state),
          child: MouseRegion(
            onHover: (event) => _handleHover(event, state),
            child: CustomPaint(
              size: Size.infinite,
              painter: GraphPainter(
                nodes: state.nodes,
                edges: state.edges,
                selectedNodeId: state.selectedNodeId,
                hoveredNodeId: _hoveredNodeId,
                offset: _offset,
                scale: _scale,
                isLowGpuMode: state.isLowGpuMode,
                interactionSpeed: _interactionSpeed,
              ),
            ),
          ),
        ),
      ],
    );
  }
  
  void _handleTap(TapUpDetails details, GraphState state) {
    final localPos = details.localPosition;
    final node = _hitTest(localPos, state.nodes);
    
    if (node != null) {
      ref.read(graphProvider.notifier).selectNode(node.id);
    } else {
      ref.read(graphProvider.notifier).selectNode(null);
    }
  }

  void _handleHover(PointerEvent event, GraphState state) {
    final localPos = event.localPosition;
    final node = _hitTest(localPos, state.nodes);
    
    if (node?.id != _hoveredNodeId) {
      setState(() {
        _hoveredNodeId = node?.id;
      });
    }
  }
  
  GraphNode? _hitTest(Offset localPos, List<GraphNode> nodes) {
    // Inverse transform the touch point to graph coordinates
    // graphPoint = (screenPoint - offset) / scale
    final graphX = (localPos.dx - _offset.dx) / _scale;
    final graphY = (localPos.dy - _offset.dy) / _scale;
    
    // Brute force hit test (reverse order to hit top nodes first)
    for (var i = nodes.length - 1; i >= 0; i--) {
       final node = nodes[i];
       final dx = graphX - node.x;
       final dy = graphY - node.y;
       final r = 20.0; // Approximation of radius
       
       if (dx * dx + dy * dy < r * r) {
         return node;
       }
    }
    return null;
  }

  void _zoomToFit(List<GraphNode> nodes, Size screenSize) {
    if (nodes.isEmpty) return;
    
    // Calculate Bounds
    double minX = double.infinity, maxX = -double.infinity;
    double minY = double.infinity, maxY = -double.infinity;
    
    for (var n in nodes) {
      if (n.x < minX) minX = n.x;
      if (n.x > maxX) maxX = n.x;
      if (n.y < minY) minY = n.y;
      if (n.y > maxY) maxY = n.y;
    }
    
    final w = maxX - minX;
    final h = maxY - minY;
    if (w == 0 || h == 0) return;
    
    // Fit
    final padding = 100.0;
    final scaleX = (screenSize.width - padding * 2) / w;
    final scaleY = (screenSize.height - padding * 2) / h;
    final targetScale = (scaleX < scaleY ? scaleX : scaleY).clamp(0.1, 2.0);
    
    final centerX = (minX + maxX) / 2;
    final centerY = (minY + maxY) / 2;
    
    final screenCenterX = screenSize.width / 2;
    final screenCenterY = screenSize.height / 2;
    
    // Offset = ScreenCenter - GraphCenter * scale
    // _transformPoint logic: x * scale + offset
    // offset = ScreenPoint - GraphPoint * scale
    final targetOffsetX = screenCenterX - centerX * targetScale;
    final targetOffsetY = screenCenterY - centerY * targetScale;
    
    setState(() {
      _scale = targetScale;
      _offset = Offset(targetOffsetX, targetOffsetY);
    });
  }

  /// Animates the camera smoothly to focus on a specific node (AI Spotlight).
  void _animateToNode(String nodeId, List<GraphNode> nodes, Size screenSize) {
    final node = nodes.cast<GraphNode?>().firstWhere(
      (n) => n?.id == nodeId,
      orElse: () => null,
    );
    
    if (node == null) return;
    
    // Store starting state for interpolation
    _startOffset = _offset;
    _startScale = _scale;
    
    // Calculate target state (center node on screen with nice zoom)
    _targetScale = 1.8;
    final screenCenterX = screenSize.width / 2;
    final screenCenterY = screenSize.height / 2;
    _targetOffset = Offset(
      screenCenterX - node.x * _targetScale,
      screenCenterY - node.y * _targetScale,
    );
    
    // Trigger smooth animation
    _cameraAnimController.forward(from: 0.0);
    
    // Haptic feedback for spotlight
    HapticFeedback.mediumImpact();
  }
}
